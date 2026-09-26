//! Bounded admission of deferrable work. Budgets are estimates, never hard
//! execution-time guarantees. Network events, gameplay, and mandatory HUD state
//! must not be submitted here. A renderer applies Publish results on its owning
//! thread before constructing its next immutable snapshot.

use crate::config::{ConfigError, EventHorizonConfig, SchedulingMode};
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorldRevision {
    pub world: u64,
    pub resources: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct JobKey(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobKind {
    MeshUpload,
    AssetUpload,
    WasmCallback,
    NativeCallback,
}
impl JobKind {
    fn index(self) -> usize {
        match self {
            Self::MeshUpload => 0,
            Self::AssetUpload => 1,
            Self::WasmCallback => 2,
            Self::NativeCallback => 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub key: JobKey,
    /// Monotonic per-key content revision; distinct from world/resource epochs.
    pub revision: u64,
    pub epoch: WorldRevision,
    pub kind: JobKind,
    pub enqueued_at_us: u64,
    pub nearby_visible: bool,
    pub distance_squared: u64,
    pub estimated_cpu_us: u64,
    pub estimated_gpu_us: u64,
    /// All externally owned buffers kept alive by this job must be charged here.
    /// Do not count total world memory; this is additional scheduling storage.
    pub resident_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameBudget {
    pub now_us: u64,
    /// Available headroom *after* required gameplay and rendering work.
    pub cpu_us: u64,
    pub gpu_us: u64,
}

#[derive(Clone, Debug)]
pub struct AdmittedJob {
    pub ticket: u64,
    pub job: Job,
    pub estimated_cpu_us: u64,
    pub estimated_gpu_us: u64,
    pub deadline_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completion {
    Publish,
    DiscardStale,
    UnknownTicket,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Enqueued {
    Added,
    Coalesced,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EnqueueError {
    #[error("job's world/resource epoch is stale")]
    StaleEpoch,
    #[error("job revision is older than or duplicates current work")]
    StaleRevision,
    #[error("scheduler job-count ceiling reached")]
    QueueFull,
    #[error("scheduler memory budget would be exceeded")]
    MemoryBudget,
    #[error("job byte accounting overflowed")]
    AccountingOverflow,
}

#[derive(Clone, Debug, Default)]
pub struct SchedulerStats {
    pub enqueued: u64,
    pub coalesced: u64,
    pub admitted: u64,
    pub published: u64,
    pub discarded_stale: u64,
    pub missed_deadlines: u64,
    pub cost_overruns: u64,
    pub rejected: u64,
    pub fairness_admissions: u64,
    pub oldest_wait_us: u64,
}

#[derive(Default, Clone, Copy)]
struct Estimate {
    mean: u64,
    deviation: u64,
    initialized: bool,
}
impl Estimate {
    fn observe(&mut self, sample: u64) {
        if !self.initialized {
            self.mean = sample;
            self.initialized = true;
            return;
        }
        let difference = self.mean.abs_diff(sample);
        self.mean = ((u128::from(self.mean) * 7 + u128::from(sample)) / 8) as u64;
        self.deviation = ((u128::from(self.deviation) * 7 + u128::from(difference)) / 8) as u64;
    }
    fn conservative(self) -> u64 {
        self.mean.saturating_add(self.deviation.saturating_mul(2))
    }
}

#[derive(Default, Clone, Copy)]
struct CostHistory {
    cpu: Estimate,
    gpu: Estimate,
}

struct Pending {
    job: Job,
    deadline_us: u64,
    deadline_recorded: bool,
}
struct Running {
    admitted: AdmittedJob,
    deadline_recorded: bool,
}
#[derive(Default)]
struct KeyState {
    newest: u64,
    running: usize,
}

pub struct Scheduler {
    settings: EventHorizonConfig,
    epoch: WorldRevision,
    frame_period_us: u64,
    pending: BTreeMap<JobKey, Pending>,
    running: HashMap<u64, Running>,
    keys: HashMap<JobKey, KeyState>,
    costs: [CostHistory; 4],
    bytes: u64,
    next_ticket: u64,
    stats: SchedulerStats,
}

impl Scheduler {
    /// Conservative charge for descriptor/map/ticket overhead in addition to
    /// caller-charged payloads. The count ceiling also bounds allocator metadata.
    const DESCRIPTOR_BYTES: u64 = 512;

    pub fn new(settings: EventHorizonConfig, target_fps: u32) -> Result<Self, ConfigError> {
        settings.validate()?;
        if target_fps > 2000 {
            return Err(ConfigError::Invalid("target_fps exceeds 2000".into()));
        }
        let rate = if target_fps == 0 { 240 } else { target_fps };
        Ok(Self {
            settings,
            epoch: WorldRevision::default(),
            frame_period_us: 1_000_000 / u64::from(rate),
            pending: BTreeMap::new(),
            running: HashMap::new(),
            keys: HashMap::new(),
            costs: [CostHistory::default(); 4],
            bytes: 0,
            next_ticket: 1,
            stats: SchedulerStats::default(),
        })
    }

    pub fn mode(&self) -> SchedulingMode {
        self.settings.mode
    }
    pub fn epoch(&self) -> WorldRevision {
        self.epoch
    }
    pub fn stats(&self) -> &SchedulerStats {
        &self.stats
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn in_flight_count(&self) -> usize {
        self.running.len()
    }
    pub fn resident_bytes(&self) -> u64 {
        self.bytes
    }

    /// A non-consuming publication precheck for a synchronous upload on the
    /// scheduler's owner thread. Report actual execution time with complete
    /// afterward. No authoritative mutation may interleave this check and the
    /// publication; asynchronous upload results must still check on completion.
    pub fn is_current(&self, ticket: u64) -> bool {
        self.running.get(&ticket).is_some_and(|running| {
            let job = &running.admitted.job;
            job.epoch == self.epoch
                && self
                    .keys
                    .get(&job.key)
                    .is_some_and(|state| state.newest == job.revision)
        })
    }

    /// Call as soon as authoritative content changes, before allocating or
    /// enqueueing its replacement. Even when replacement enqueueing fails under
    /// backpressure, older workers can no longer publish stale content.
    pub fn invalidate_content(&mut self, key: JobKey, revision: u64) {
        let obsolete = self
            .pending
            .get(&key)
            .is_some_and(|entry| entry.job.revision < revision);
        if obsolete && let Some(entry) = self.pending.remove(&key) {
            self.bytes -= entry.job.resident_bytes + Self::DESCRIPTOR_BYTES;
            self.stats.discarded_stale = self.stats.discarded_stale.saturating_add(1);
        }
        if let Some(state) = self.keys.get_mut(&key) {
            state.newest = state.newest.max(revision);
            if state.running == 0 && !self.pending.contains_key(&key) {
                self.keys.remove(&key);
            }
        }
    }

    /// A dimension/resource reload invalidates queued and running work. Running
    /// storage remains charged until workers report completion/cancellation.
    pub fn set_epoch(&mut self, epoch: WorldRevision) {
        if epoch == self.epoch {
            return;
        }
        self.epoch = epoch;
        for entry in self.pending.values() {
            self.bytes -= entry.job.resident_bytes + Self::DESCRIPTOR_BYTES;
            self.stats.discarded_stale = self.stats.discarded_stale.saturating_add(1);
        }
        self.pending.clear();
        self.keys.retain(|_, state| state.running != 0);
        // Revisions are scoped to an epoch; a new world may start from zero.
        for state in self.keys.values_mut() {
            state.newest = 0;
        }
    }

    pub fn enqueue(&mut self, job: Job) -> Result<Enqueued, EnqueueError> {
        let result = self.enqueue_inner(job);
        if result.is_err() {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
        }
        result
    }

    fn enqueue_inner(&mut self, job: Job) -> Result<Enqueued, EnqueueError> {
        if job.epoch != self.epoch {
            return Err(EnqueueError::StaleEpoch);
        }
        // Old-epoch workers must not prevent a new world using revision zero.
        let current_key_exists = self.pending.contains_key(&job.key)
            || self
                .running
                .values()
                .any(|r| r.admitted.job.key == job.key && r.admitted.job.epoch == self.epoch);
        let duplicates_existing = self
            .pending
            .get(&job.key)
            .is_some_and(|p| p.job.revision == job.revision)
            || self.running.values().any(|r| {
                r.admitted.job.key == job.key
                    && r.admitted.job.epoch == self.epoch
                    && r.admitted.job.revision == job.revision
            });
        if current_key_exists
            && (self
                .keys
                .get(&job.key)
                .is_some_and(|state| job.revision < state.newest)
                || duplicates_existing)
        {
            return Err(EnqueueError::StaleRevision);
        }
        let previous = self.pending.get(&job.key);
        if previous.is_none() && self.pending.len() + self.running.len() >= self.settings.max_jobs {
            return Err(EnqueueError::QueueFull);
        }
        let charge = job
            .resident_bytes
            .checked_add(Self::DESCRIPTOR_BYTES)
            .ok_or(EnqueueError::AccountingOverflow)?;
        let released =
            previous.map_or(0, |entry| entry.job.resident_bytes + Self::DESCRIPTOR_BYTES);
        let next_bytes = self
            .bytes
            .checked_sub(released)
            .and_then(|v| v.checked_add(charge))
            .ok_or(EnqueueError::AccountingOverflow)?;
        let memory_ceiling = u64::from(self.settings.memory_budget_mib) * 1024 * 1024;
        if next_bytes > memory_ceiling {
            return Err(EnqueueError::MemoryBudget);
        }
        let result = if previous.is_some() {
            Enqueued::Coalesced
        } else {
            Enqueued::Added
        };
        // Preserve original queue age through coalescing to avoid starvation
        // when a frequently changed section continually replaces its mesh job.
        let enqueued_at_us = previous.map_or(job.enqueued_at_us, |p| {
            p.job.enqueued_at_us.min(job.enqueued_at_us)
        });
        let job = Job {
            enqueued_at_us,
            ..job
        };
        let delay = if job.nearby_visible {
            self.frame_period_us
                .saturating_mul(u64::from(self.settings.nearby_deadline_frames))
        } else {
            u64::from(self.settings.aging_ms) * 1000
        };
        let deadline_us = enqueued_at_us.saturating_add(delay);
        self.keys.entry(job.key).or_default().newest = job.revision;
        self.pending.insert(
            job.key,
            Pending {
                job,
                deadline_us,
                deadline_recorded: false,
            },
        );
        self.bytes = next_bytes;
        self.stats.enqueued = self.stats.enqueued.saturating_add(1);
        if result == Enqueued::Coalesced {
            self.stats.coalesced = self.stats.coalesced.saturating_add(1);
        }
        Ok(result)
    }

    fn estimated_cost(&self, job: &Job) -> (u64, u64) {
        if self.settings.mode != SchedulingMode::EventHorizon {
            return (job.estimated_cpu_us, job.estimated_gpu_us);
        }
        let history = self.costs[job.kind.index()];
        (
            job.estimated_cpu_us.max(history.cpu.conservative()),
            job.estimated_gpu_us.max(history.gpu.conservative()),
        )
    }

    /// Pick work without waiting for workers or GPU fences. Callers must split
    /// oversized work into bounded batches; tasks that cannot fit stay queued.
    pub fn admit(&mut self, budget: FrameBudget) -> Vec<AdmittedJob> {
        self.record_deadlines(budget.now_us);
        let horizon = self.settings.mode == SchedulingMode::EventHorizon;
        let margin = if horizon {
            u64::from(self.settings.safety_margin_us)
        } else {
            0
        };
        let mut cpu = budget.cpu_us.saturating_sub(margin);
        let mut gpu = budget.gpu_us.saturating_sub(margin);
        let mut order: Vec<_> = self.pending.keys().copied().collect();
        order.sort_by_key(|key| {
            let pending = &self.pending[key];
            let job = &pending.job;
            let expired = horizon && pending.deadline_us <= budget.now_us;
            (
                if expired {
                    0
                } else if job.nearby_visible {
                    1
                } else {
                    2
                },
                if expired { pending.deadline_us } else { 0 },
                job.distance_squared,
                job.enqueued_at_us,
                *key,
            )
        });
        let mut selected = Vec::new();
        if horizon {
            let reserve_cpu =
                (u128::from(cpu) * u128::from(self.settings.fairness_reserve_percent) / 100) as u64;
            let reserve_gpu =
                (u128::from(gpu) * u128::from(self.settings.fairness_reserve_percent) / 100) as u64;
            let oldest = order
                .iter()
                .filter(|key| {
                    let job = &self.pending[key].job;
                    let cost = self.estimated_cost(job);
                    budget.now_us.saturating_sub(job.enqueued_at_us)
                        >= u64::from(self.settings.aging_ms) * 1000
                        && cost.0 <= reserve_cpu
                        && cost.1 <= reserve_gpu
                })
                .min_by_key(|key| (self.pending[key].job.enqueued_at_us, **key))
                .copied();
            if let Some(key) = oldest {
                if let Some(admitted) = self.take(key, &mut cpu, &mut gpu) {
                    selected.push(admitted);
                    self.stats.fairness_admissions =
                        self.stats.fairness_admissions.saturating_add(1);
                }
                order.retain(|candidate| *candidate != key);
            }
        }
        for key in order {
            if let Some(admitted) = self.take(key, &mut cpu, &mut gpu) {
                selected.push(admitted);
            }
        }
        selected
    }

    fn take(&mut self, key: JobKey, cpu: &mut u64, gpu: &mut u64) -> Option<AdmittedJob> {
        let entry = self.pending.get(&key)?;
        let (estimated_cpu_us, estimated_gpu_us) = self.estimated_cost(&entry.job);
        if estimated_cpu_us > *cpu || estimated_gpu_us > *gpu {
            return None;
        }
        // Never wrap a ticket and accidentally complete an unrelated job.
        let next_ticket = self.next_ticket.checked_add(1)?;
        let entry = self.pending.remove(&key)?;
        let admitted = AdmittedJob {
            ticket: self.next_ticket,
            job: entry.job,
            estimated_cpu_us,
            estimated_gpu_us,
            deadline_us: entry.deadline_us,
        };
        self.next_ticket = next_ticket;
        self.keys.get_mut(&key)?.running += 1;
        self.running.insert(
            admitted.ticket,
            Running {
                admitted: admitted.clone(),
                deadline_recorded: entry.deadline_recorded,
            },
        );
        *cpu -= estimated_cpu_us;
        *gpu -= estimated_gpu_us;
        self.stats.admitted = self.stats.admitted.saturating_add(1);
        Some(admitted)
    }

    pub fn complete(
        &mut self,
        ticket: u64,
        now_us: u64,
        actual_cpu_us: u64,
        actual_gpu_us: u64,
    ) -> Completion {
        let Some(running) = self.running.remove(&ticket) else {
            return Completion::UnknownTicket;
        };
        let admitted = running.admitted;
        let job = &admitted.job;
        self.bytes -= job.resident_bytes + Self::DESCRIPTOR_BYTES;
        let cost = &mut self.costs[job.kind.index()];
        cost.cpu.observe(actual_cpu_us);
        cost.gpu.observe(actual_gpu_us);
        if actual_cpu_us > admitted.estimated_cpu_us || actual_gpu_us > admitted.estimated_gpu_us {
            self.stats.cost_overruns = self.stats.cost_overruns.saturating_add(1);
        }
        if now_us > admitted.deadline_us && !running.deadline_recorded {
            self.stats.missed_deadlines = self.stats.missed_deadlines.saturating_add(1);
        }
        let state = self
            .keys
            .get_mut(&job.key)
            .expect("running key has accounting state");
        let stale = job.epoch != self.epoch || state.newest != job.revision;
        state.running -= 1;
        if state.running == 0 && !self.pending.contains_key(&job.key) {
            self.keys.remove(&job.key);
        }
        if stale {
            self.stats.discarded_stale = self.stats.discarded_stale.saturating_add(1);
            Completion::DiscardStale
        } else {
            self.stats.published = self.stats.published.saturating_add(1);
            Completion::Publish
        }
    }

    /// Cancellation releases storage only once the worker has actually stopped.
    pub fn cancel_completed(&mut self, ticket: u64) -> bool {
        let Some(running) = self.running.remove(&ticket) else {
            return false;
        };
        let job = running.admitted.job;
        self.bytes -= job.resident_bytes + Self::DESCRIPTOR_BYTES;
        let state = self
            .keys
            .get_mut(&job.key)
            .expect("running key has accounting state");
        state.running -= 1;
        if state.running == 0 && !self.pending.contains_key(&job.key) {
            self.keys.remove(&job.key);
        }
        self.stats.discarded_stale = self.stats.discarded_stale.saturating_add(1);
        true
    }

    fn record_deadlines(&mut self, now_us: u64) {
        self.stats.oldest_wait_us = self
            .pending
            .values()
            .map(|p| now_us.saturating_sub(p.job.enqueued_at_us))
            .max()
            .unwrap_or(0);
        for pending in self.pending.values_mut() {
            if now_us > pending.deadline_us && !pending.deadline_recorded {
                pending.deadline_recorded = true;
                self.stats.missed_deadlines = self.stats.missed_deadlines.saturating_add(1);
            }
        }
        for running in self.running.values_mut() {
            if now_us > running.admitted.deadline_us && !running.deadline_recorded {
                running.deadline_recorded = true;
                self.stats.missed_deadlines = self.stats.missed_deadlines.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scheduler(mode: SchedulingMode) -> Scheduler {
        Scheduler::new(
            EventHorizonConfig {
                mode,
                safety_margin_us: 0,
                ..Default::default()
            },
            240,
        )
        .unwrap()
    }
    fn job(key: u64) -> Job {
        Job {
            key: JobKey(key),
            revision: 1,
            epoch: WorldRevision::default(),
            kind: JobKind::MeshUpload,
            enqueued_at_us: 0,
            nearby_visible: false,
            distance_squared: 100,
            estimated_cpu_us: 50,
            estimated_gpu_us: 20,
            resident_bytes: 100,
        }
    }
    fn budget(now_us: u64, cpu_us: u64, gpu_us: u64) -> FrameBudget {
        FrameBudget {
            now_us,
            cpu_us,
            gpu_us,
        }
    }
    #[test]
    fn cpu_and_gpu_are_independent_admission_limits() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        for key in 0..3 {
            s.enqueue(job(key)).unwrap();
        }
        let admitted = s.admit(budget(0, 500, 25));
        assert_eq!(admitted.len(), 1);
        assert_eq!(s.pending_count(), 2);
        assert!(s.admit(budget(0, 49, 100)).is_empty());
    }
    #[test]
    fn coalescing_does_not_reset_age_and_rejects_obsolete_completion() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(job(1)).unwrap();
        let old = s.admit(budget(0, 100, 100)).remove(0);
        s.enqueue(Job {
            revision: 2,
            ..job(1)
        })
        .unwrap();
        s.enqueue(Job {
            revision: 3,
            enqueued_at_us: 500_000,
            ..job(1)
        })
        .unwrap();
        let new = s.admit(budget(500_000, 100, 100)).remove(0);
        assert_eq!(new.job.enqueued_at_us, 0);
        // Completing the newest job first must not let the old one publish.
        assert_eq!(s.complete(new.ticket, 500_010, 50, 20), Completion::Publish);
        assert_eq!(
            s.complete(old.ticket, 500_011, 50, 20),
            Completion::DiscardStale
        );
        assert_eq!(s.resident_bytes(), 0);
        assert!(s.keys.is_empty());
    }
    #[test]
    fn memory_and_count_bounds_include_running_jobs() {
        let mut s = Scheduler::new(
            EventHorizonConfig {
                max_jobs: 1,
                memory_budget_mib: 1,
                ..Default::default()
            },
            240,
        )
        .unwrap();
        s.enqueue(job(1)).unwrap();
        let old = s.admit(budget(0, 1000, 1000)).remove(0);
        assert_eq!(s.enqueue(job(2)), Err(EnqueueError::QueueFull));
        assert!(s.cancel_completed(old.ticket));
        assert_eq!(
            s.enqueue(Job {
                resident_bytes: u64::MAX,
                ..job(3)
            }),
            Err(EnqueueError::AccountingOverflow)
        );
        assert_eq!(
            s.enqueue(Job {
                resident_bytes: 1024 * 1024,
                ..job(3)
            }),
            Err(EnqueueError::MemoryBudget)
        );
        assert_eq!(s.resident_bytes(), 0);
    }
    #[test]
    fn failed_replacement_retains_old_job() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(job(1)).unwrap();
        let bytes = s.resident_bytes();
        assert_eq!(
            s.enqueue(Job {
                revision: 2,
                resident_bytes: u64::MAX,
                ..job(1)
            }),
            Err(EnqueueError::AccountingOverflow)
        );
        assert_eq!(s.resident_bytes(), bytes);
        assert_eq!(s.admit(budget(0, 1000, 1000))[0].job.revision, 1);
    }

    #[test]
    fn content_invalidation_prevents_stale_publication_if_replacement_is_rejected() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(job(1)).unwrap();
        let old = s.admit(budget(0, 1000, 1000)).remove(0);
        assert!(s.is_current(old.ticket));
        s.invalidate_content(JobKey(1), 2);
        assert!(!s.is_current(old.ticket));
        assert_eq!(
            s.enqueue(Job {
                revision: 2,
                resident_bytes: u64::MAX,
                ..job(1)
            }),
            Err(EnqueueError::AccountingOverflow)
        );
        assert_eq!(
            s.complete(old.ticket, 100, 50, 20),
            Completion::DiscardStale
        );
        s.enqueue(Job {
            revision: 2,
            ..job(1)
        })
        .unwrap();
        let next = s.admit(budget(0, 1000, 1000)).remove(0);
        assert_eq!(s.complete(next.ticket, 100, 50, 20), Completion::Publish);
    }
    #[test]
    fn aging_gives_distant_jobs_service_under_nearby_load() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(job(1)).unwrap();
        for key in 2..12 {
            s.enqueue(Job {
                enqueued_at_us: 249_000,
                nearby_visible: true,
                distance_squared: 0,
                ..job(key)
            })
            .unwrap();
        }
        let admitted = s.admit(budget(250_000, 500, 200));
        assert_eq!(admitted[0].job.key, JobKey(1));
        assert_eq!(s.stats().fairness_admissions, 1);
    }
    #[test]
    fn epochs_cancel_old_work_without_releasing_live_worker_memory() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(job(1)).unwrap();
        let old = s.admit(budget(0, 1000, 1000)).remove(0);
        let epoch = WorldRevision {
            world: 2,
            resources: 0,
        };
        s.set_epoch(epoch);
        assert!(s.resident_bytes() > 0);
        s.enqueue(Job {
            epoch,
            revision: 0,
            ..job(1)
        })
        .unwrap();
        assert_eq!(s.complete(old.ticket, 10, 50, 20), Completion::DiscardStale);
        let new = s.admit(budget(0, 1000, 1000)).remove(0);
        assert_eq!(s.complete(new.ticket, 10, 50, 20), Completion::Publish);
        assert_eq!(s.resident_bytes(), 0);
    }
    #[test]
    fn deadlines_count_once_and_cost_history_adapts() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(job(1)).unwrap();
        let ticket = s.admit(budget(300_000, 1000, 1000))[0].ticket;
        s.admit(budget(400_000, 0, 0));
        s.complete(ticket, 500_000, 300, 100);
        assert_eq!(s.stats().missed_deadlines, 1);
        assert_eq!(s.stats().cost_overruns, 1);
        s.enqueue(job(2)).unwrap();
        assert!(s.admit(budget(500_000, 299, 1000)).is_empty());
    }
    #[test]
    fn extreme_clocks_and_estimates_do_not_wrap() {
        let mut s = scheduler(SchedulingMode::EventHorizon);
        s.enqueue(Job {
            enqueued_at_us: u64::MAX,
            estimated_cpu_us: u64::MAX,
            ..job(1)
        })
        .unwrap();
        assert!(s.admit(budget(0, 1000, 1000)).is_empty());
        assert_eq!(s.stats().oldest_wait_us, 0);
        let work = s.admit(budget(u64::MAX, u64::MAX, u64::MAX));
        assert_eq!(work.len(), 1);
        s.complete(work[0].ticket, u64::MAX, u64::MAX, u64::MAX);
    }
}
