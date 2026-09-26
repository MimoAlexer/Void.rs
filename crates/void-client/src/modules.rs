use crossbeam_channel::{Receiver, Sender, bounded};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use void_core::{Job, JobKey, JobKind, Scheduler};
use void_sdk::{FrameMetrics, HudText, MAX_HUD_COMMANDS, MAX_HUD_TEXT_BYTES};
use void_services::mods::{ModLimits, ModManifest, NativeMod, Runtime, WasmMod};

const FIRST_MOD_KEY: u64 = 100;
const MAX_MODS: usize = 256;
const CALLBACK_STORAGE: u64 =
    (MAX_HUD_COMMANDS * (MAX_HUD_TEXT_BYTES + std::mem::size_of::<HudText>())) as u64;

pub struct Available {
    pub manifest: ModManifest,
    pub directory: PathBuf,
    pub trust_native: bool,
}
enum Instance {
    Wasm(Box<WasmMod>),
    Native(NativeMod),
    #[cfg(test)]
    Test {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
    },
}
impl Instance {
    fn frame(&mut self, metrics: FrameMetrics) -> Result<Vec<HudText>, String> {
        match self {
            Self::Wasm(module) => module
                .on_frame(metrics)
                .map_err(|error| format!("{error:#}")),
            Self::Native(module) => module
                .on_frame(metrics)
                .map_err(|error| format!("{error:#}")),
            #[cfg(test)]
            Self::Test { calls, fail } => {
                calls.fetch_add(1, Ordering::Relaxed);
                if *fail {
                    Err("test callback failed".into())
                } else {
                    Ok(vec![HudText {
                        x: 0.0,
                        y: 0.0,
                        rgba: u32::MAX,
                        text: "test".into(),
                    }])
                }
            }
        }
    }
}
struct Loaded {
    manifest: ModManifest,
    instance: Instance,
    hud: Vec<HudText>,
    error: Option<String>,
    enabled: bool,
    key: JobKey,
    revision: u64,
    estimated_cpu_us: u64,
    fingerprint: Option<String>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Selection {
    enabled: bool,
    trusted_native_sha256: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    schema: u32,
    #[serde(default)]
    modules: BTreeMap<String, Selection>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema: 1,
            modules: BTreeMap::new(),
        }
    }
}
struct LoadResult {
    request: u64,
    id: String,
    result: Result<Loaded, String>,
}
struct PendingLoad {
    request: u64,
    id: String,
    cancelled: bool,
}
enum NativePermission {
    Explicit,
    Saved(String),
    None,
}

pub struct ModHost {
    pub directory: PathBuf,
    pub available: Vec<Available>,
    pub notice: String,
    pub loading: bool,
    pub callback_overruns: u64,
    loaded: Vec<Loaded>,
    tx: Sender<LoadResult>,
    rx: Receiver<LoadResult>,
    queued: HashMap<JobKey, u64>,
    invalidate: Vec<(JobKey, u64)>,
    preferences: Preferences,
    persistence_valid: bool,
    restore: VecDeque<String>,
    pending: Option<PendingLoad>,
    next_request: u64,
    next_key: u64,
    frame_clock: Option<(u64, Instant)>,
}
impl ModHost {
    pub fn new(directory: PathBuf) -> Self {
        let (tx, rx) = bounded(4);
        let (preferences, persistence_valid, error) =
            match read_preferences(&directory.join("enabled.toml")) {
                Ok(preferences) => (preferences, true, None),
                Err(error) => (Preferences::default(), false, Some(error)),
            };
        let mut host = Self {
            directory,
            available: Vec::new(),
            notice: String::new(),
            loading: false,
            callback_overruns: 0,
            loaded: Vec::new(),
            tx,
            rx,
            queued: HashMap::new(),
            invalidate: Vec::new(),
            preferences,
            persistence_valid,
            restore: VecDeque::new(),
            pending: None,
            next_request: 1,
            next_key: FIRST_MOD_KEY,
            frame_clock: None,
        };
        host.scan();
        if let Some(error) = error {
            host.notice = format!("Mod selection file was retained unchanged: {error}");
        }
        host
    }
    pub fn scan(&mut self) {
        self.available.clear();
        self.restore.clear();
        self.notice.clear();
        if !self.persistence_valid
            && let Ok(preferences) = read_preferences(&self.directory.join("enabled.toml"))
        {
            self.preferences = preferences;
            self.persistence_valid = true;
        }
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.notice =
                    "Create this folder and add a subfolder containing mod.toml and its module."
                        .into();
                return;
            }
            Err(error) => {
                self.notice = error.to_string();
                return;
            }
        };
        let mut ids = HashSet::new();
        let mut duplicates = HashSet::new();
        for entry in entries.flatten().take(MAX_MODS + 1) {
            let directory = entry.path();
            let path = directory.join("mod.toml");
            if !path.is_file() {
                continue;
            }
            if self.available.len() >= MAX_MODS {
                self.notice = "Mod discovery is limited to 256 modules.".into();
                break;
            }
            let parsed = (|| -> anyhow::Result<ModManifest> {
                anyhow::ensure!(
                    path.metadata()?.len() <= 64 * 1024,
                    "manifest exceeds 64 KiB"
                );
                ModManifest::read(&path)
            })();
            match parsed {
                Ok(manifest) => {
                    if !ids.insert(manifest.id.clone()) {
                        duplicates.insert(manifest.id.clone());
                    }
                    let trust_native = self.loaded.iter().any(|module| {
                        module.manifest.id == manifest.id && module.fingerprint.is_some()
                    });
                    self.available.push(Available {
                        manifest,
                        directory,
                        trust_native,
                    });
                }
                Err(error) => self.notice = format!("{}: {error:#}", path.display()),
            }
        }
        if !duplicates.is_empty() {
            self.available
                .retain(|item| !duplicates.contains(&item.manifest.id));
            self.notice =
                "Duplicate mod IDs were excluded; installed modules need unique IDs.".into();
        }
        self.available
            .sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        for item in &self.available {
            if self
                .preferences
                .modules
                .get(&item.manifest.id)
                .is_some_and(|selection| selection.enabled)
                && !self
                    .loaded
                    .iter()
                    .any(|module| module.manifest.id == item.manifest.id)
                && !self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.id == item.manifest.id)
            {
                self.restore.push_back(item.manifest.id.clone());
            }
        }
    }
    pub fn enabled(&self, id: &str) -> bool {
        self.loaded
            .iter()
            .any(|module| module.manifest.id == id && module.enabled && module.error.is_none())
    }
    pub fn error(&self, id: &str) -> Option<&str> {
        self.loaded
            .iter()
            .find(|module| module.manifest.id == id)
            .and_then(|module| module.error.as_deref())
    }
    /// Stop callbacks immediately; keep native code loaded until process exit.
    pub fn disable(&mut self, id: &str) {
        self.restore.retain(|candidate| candidate != id);
        if let Some(pending) = self.pending.as_mut().filter(|pending| pending.id == id) {
            pending.cancelled = true;
        }
        if let Some(module) = self
            .loaded
            .iter_mut()
            .find(|module| module.manifest.id == id)
        {
            module.enabled = false;
            module.hud.clear();
            module.revision = module.revision.saturating_add(1);
            self.invalidate.push((module.key, module.revision));
            self.queued.remove(&module.key);
        }
        self.preferences
            .modules
            .entry(id.into())
            .or_default()
            .enabled = false;
        self.notice = format!("Disabled {id}");
        self.persist();
    }
    pub fn enable(&mut self, index: usize) {
        self.begin_enable(index, false);
    }
    fn begin_enable(&mut self, index: usize, restoring: bool) {
        if self.loading {
            return;
        }
        let Some(available) = self.available.get(index) else {
            return;
        };
        let id = available.manifest.id.clone();
        if self.enabled(&id) {
            return;
        }
        if let Some(module) = self
            .loaded
            .iter_mut()
            .find(|module| module.manifest.id == id)
        {
            if module.error.is_none() {
                // Resuming an instance never loads replacement native bytes.
                module.enabled = true;
                self.preferences
                    .modules
                    .entry(id.clone())
                    .or_default()
                    .enabled = true;
                self.notice = format!("Resumed {id}");
                self.persist();
                return;
            }
            if module.manifest.runtime == Runtime::Native {
                self.notice = format!("{id} failed. Restart before loading native code again.");
                return;
            }
        }
        let permission = if available.manifest.runtime == Runtime::Native {
            if !restoring && available.trust_native {
                NativePermission::Explicit
            } else if restoring {
                match self
                    .preferences
                    .modules
                    .get(&id)
                    .and_then(|selection| selection.trusted_native_sha256.clone())
                {
                    Some(fingerprint) => NativePermission::Saved(fingerprint),
                    None => {
                        self.notice = format!("{id} needs an explicit native-code trust choice.");
                        return;
                    }
                }
            } else {
                self.notice = "Explicitly trust this native module before loading it.".into();
                return;
            }
        } else {
            NativePermission::None
        };
        let Some(next_request) = self.next_request.checked_add(1) else {
            self.notice = "Mod request counter exhausted; restart.".into();
            return;
        };
        let Some(next_key) = self.next_key.checked_add(1) else {
            self.notice = "Mod scheduler key counter exhausted; restart.".into();
            return;
        };
        let request = self.next_request;
        let key = JobKey(self.next_key);
        self.next_request = next_request;
        self.next_key = next_key;
        let manifest = available.manifest.clone();
        let directory = available.directory.clone();
        let tx = self.tx.clone();
        self.restore.retain(|candidate| candidate != &id);
        self.pending = Some(PendingLoad {
            request,
            id: id.clone(),
            cancelled: false,
        });
        self.loading = true;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                load_instance(&directory, manifest, permission, key)
                    .map_err(|error| format!("{error:#}"))
            }))
            .unwrap_or_else(|_| Err("Mod loader worker panicked; the module was disabled.".into()));
            let _ = tx.send(LoadResult {
                request,
                id,
                result,
            });
        });
    }
    pub fn poll(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            let Some(pending) = self.pending.take() else {
                continue;
            };
            if pending.request != message.request {
                self.pending = Some(pending);
                continue;
            }
            self.loading = false;
            match message.result {
                Ok(mut module) => {
                    // Even a cancelled native library remains resident until exit.
                    module.enabled = !pending.cancelled;
                    let selection = self
                        .preferences
                        .modules
                        .entry(message.id.clone())
                        .or_default();
                    selection.enabled = module.enabled;
                    selection.trusted_native_sha256 = module.fingerprint.clone();
                    if let Some(item) = self
                        .available
                        .iter_mut()
                        .find(|item| item.manifest.id == message.id)
                    {
                        item.trust_native = module.fingerprint.is_some();
                    }
                    if let Some(index) = self
                        .loaded
                        .iter()
                        .position(|old| old.manifest.id == message.id)
                    {
                        let old = &self.loaded[index];
                        self.invalidate
                            .push((old.key, old.revision.saturating_add(1)));
                        self.queued.remove(&old.key);
                        self.loaded[index] = module;
                    } else {
                        self.loaded.push(module);
                    }
                    self.notice = format!(
                        "{} {}",
                        if pending.cancelled {
                            "Disabled"
                        } else {
                            "Enabled"
                        },
                        message.id
                    );
                }
                Err(error) => {
                    let selection = self
                        .preferences
                        .modules
                        .entry(message.id.clone())
                        .or_default();
                    selection.enabled = false;
                    if let Some(item) = self
                        .available
                        .iter_mut()
                        .find(|item| item.manifest.id == message.id)
                        && item.manifest.runtime == Runtime::Native
                    {
                        item.trust_native = false;
                        selection.trusted_native_sha256 = None;
                    }
                    self.notice = format!("{}: {error}", message.id);
                }
            }
            self.persist();
        }
        if !self.loading {
            while let Some(id) = self.restore.pop_front() {
                if let Some(index) = self
                    .available
                    .iter()
                    .position(|item| item.manifest.id == id)
                {
                    self.begin_enable(index, true);
                    if self.loading {
                        break;
                    }
                }
            }
        }
    }
    pub fn prepare_frame(&mut self, metrics: FrameMetrics, scheduler: &mut Scheduler, now_us: u64) {
        self.frame_clock = Some((now_us, Instant::now()));
        self.poll();
        for (key, revision) in self.invalidate.drain(..) {
            scheduler.invalidate_content(key, revision);
        }
        let mut faults = Vec::new();
        for module in &mut self.loaded {
            if !module.enabled || module.error.is_some() {
                continue;
            }
            if !module.manifest.deferrable {
                let started = Instant::now();
                let result = module.instance.frame(metrics);
                let elapsed = elapsed_us(started);
                self.callback_overruns = self
                    .callback_overruns
                    .saturating_add(u64::from(elapsed > module.estimated_cpu_us));
                module.estimated_cpu_us = rolling_cost(module.estimated_cpu_us, elapsed);
                match result {
                    Ok(hud) => module.hud = hud,
                    Err(error) => {
                        fail_module(module, &error);
                        faults.push((module.manifest.id.clone(), error));
                    }
                }
                continue;
            }
            if self.queued.contains_key(&module.key) {
                continue;
            }
            // Reserve one revision for invalidation if this callback fails.
            let Some(revision) = module
                .revision
                .checked_add(1)
                .filter(|revision| *revision < u64::MAX)
            else {
                let error = "Mod revision counter exhausted; restart.".to_owned();
                fail_module(module, &error);
                faults.push((module.manifest.id.clone(), error));
                continue;
            };
            module.revision = revision;
            if scheduler
                .enqueue(Job {
                    key: module.key,
                    revision,
                    epoch: scheduler.epoch(),
                    kind: if module.manifest.runtime == Runtime::Wasm {
                        JobKind::WasmCallback
                    } else {
                        JobKind::NativeCallback
                    },
                    enqueued_at_us: now_us,
                    nearby_visible: true,
                    distance_squared: 0,
                    estimated_cpu_us: module.estimated_cpu_us.max(1),
                    estimated_gpu_us: 0,
                    resident_bytes: CALLBACK_STORAGE,
                })
                .is_ok()
            {
                self.queued.insert(module.key, revision);
            }
        }
        for (id, error) in faults {
            self.record_fault(&id, error);
        }
    }
    pub fn execute(
        &mut self,
        admitted: void_core::scheduler::AdmittedJob,
        metrics: FrameMetrics,
        scheduler: &mut Scheduler,
        now_us: u64,
    ) {
        if self.queued.get(&admitted.job.key) == Some(&admitted.job.revision) {
            self.queued.remove(&admitted.job.key);
        }
        let Some(index) = self
            .loaded
            .iter()
            .position(|module| module.key == admitted.job.key)
        else {
            scheduler.cancel_completed(admitted.ticket);
            return;
        };
        let module = &mut self.loaded[index];
        if !module.enabled
            || module.error.is_some()
            || module.revision != admitted.job.revision
            || !scheduler.is_current(admitted.ticket)
        {
            scheduler.cancel_completed(admitted.ticket);
            return;
        }
        let start = Instant::now();
        let result = module.instance.frame(metrics);
        let elapsed = elapsed_us(start);
        module.estimated_cpu_us = rolling_cost(module.estimated_cpu_us, elapsed);
        self.callback_overruns = self
            .callback_overruns
            .saturating_add(u64::from(elapsed > admitted.estimated_cpu_us));
        let completed_at = self
            .frame_clock
            .map(|(clock, instant)| clock.saturating_add(elapsed_us(instant)))
            .unwrap_or(now_us.saturating_add(elapsed));
        match result {
            Err(error) => {
                fail_module(module, &error);
                module.revision = module.revision.saturating_add(1);
                scheduler.invalidate_content(module.key, module.revision);
                // Sample failed callback cost without counting its result as published.
                scheduler.complete(admitted.ticket, completed_at, elapsed, 0);
                let id = module.manifest.id.clone();
                self.record_fault(&id, error);
            }
            Ok(hud) => {
                if scheduler.complete(admitted.ticket, completed_at, elapsed, 0)
                    == void_core::scheduler::Completion::Publish
                {
                    module.hud = hud;
                }
            }
        }
    }
    pub fn hud(&self) -> impl Iterator<Item = &HudText> {
        self.loaded
            .iter()
            .filter(|module| module.enabled && module.error.is_none())
            .flat_map(|module| &module.hud)
    }
    pub fn reset_jobs(&mut self) {
        self.queued.clear();
        self.invalidate.clear();
        for module in &mut self.loaded {
            module.hud.clear();
        }
    }
    fn record_fault(&mut self, id: &str, error: String) {
        self.preferences
            .modules
            .entry(id.into())
            .or_default()
            .enabled = false;
        self.notice = format!("{id} disabled: {error}");
        self.persist();
    }
    fn persist(&mut self) {
        if !self.persistence_valid {
            self.notice.push_str(
                " Selections were not saved because enabled.toml is invalid; fix that file first.",
            );
            return;
        }
        if let Err(error) =
            save_preferences(&self.directory.join("enabled.toml"), &self.preferences)
        {
            self.notice
                .push_str(&format!(" Selection save failed: {error}"));
        }
    }
}
fn fail_module(module: &mut Loaded, error: &str) {
    module.error = Some(error.into());
    module.enabled = false;
    module.hud.clear();
}
fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros().try_into().unwrap_or(u64::MAX)
}
fn rolling_cost(previous: u64, actual: u64) -> u64 {
    ((u128::from(previous) * 7 + u128::from(actual) * 2) / 8).min(u128::from(u64::MAX)) as u64
}

fn load_instance(
    directory: &Path,
    manifest: ModManifest,
    permission: NativePermission,
    key: JobKey,
) -> anyhow::Result<Loaded> {
    let path = manifest.resolve_entry(directory)?;
    let (instance, fingerprint) = match manifest.runtime {
        Runtime::Wasm => {
            let module = WasmMod::load(directory, manifest.clone(), ModLimits::default())
                .map_err(|error| anyhow::anyhow!("{error:#}"))?;
            (Instance::Wasm(Box::new(module)), None)
        }
        Runtime::Native => {
            anyhow::ensure!(
                path.metadata()?.len() <= 256 * 1024 * 1024,
                "native module exceeds 256 MiB"
            );
            let bytes = std::fs::read(&path)?;
            let fingerprint = native_fingerprint(&manifest, &bytes)?;
            authorize_native(&permission, &fingerprint)?;
            // Load a snapshot of exactly the approved bytes, not the mutable source.
            let cache = directory.join(".void-trusted").join(&fingerprint);
            let filename = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("native module has no filename"))?;
            let cached_entry = cache.join(filename);
            if std::fs::read(&cached_entry).ok().as_deref() != Some(bytes.as_slice()) {
                write_atomic(&cached_entry, &bytes)?;
            }
            let mut cached_manifest = manifest.clone();
            cached_manifest.entry = filename.into();
            // SAFETY: An explicit trust choice approved this manifest and binary,
            // or a previously recorded choice matched their complete SHA-256 hash.
            let module = unsafe { NativeMod::load_trusted(&cache, cached_manifest)? };
            (Instance::Native(module), Some(fingerprint))
        }
    };
    Ok(Loaded {
        manifest,
        instance,
        hud: Vec::new(),
        error: None,
        enabled: true,
        key,
        revision: 0,
        estimated_cpu_us: 150,
        fingerprint,
    })
}
fn native_fingerprint(manifest: &ModManifest, bytes: &[u8]) -> anyhow::Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"Void.rs native trust v1\0");
    let serialized = toml::to_string(manifest)?;
    hash.update((serialized.len() as u64).to_le_bytes());
    hash.update(serialized.as_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}
fn authorize_native(permission: &NativePermission, actual: &str) -> anyhow::Result<()> {
    match permission {
        NativePermission::Explicit => Ok(()),
        NativePermission::Saved(expected) if expected == actual => Ok(()),
        NativePermission::Saved(_) => anyhow::bail!(
            "Native code or its manifest changed. Review it and explicitly trust this version before loading."
        ),
        NativePermission::None => anyhow::bail!("Native code requires an explicit trust choice."),
    }
}
fn read_preferences(path: &Path) -> anyhow::Result<Preferences> {
    if !path.exists() {
        return Ok(Preferences::default());
    }
    anyhow::ensure!(
        path.metadata()?.len() <= 256 * 1024,
        "mod selection file exceeds 256 KiB"
    );
    let preferences: Preferences = toml::from_str(&std::fs::read_to_string(path)?)?;
    anyhow::ensure!(preferences.schema == 1, "unsupported mod selection schema");
    anyhow::ensure!(
        preferences.modules.len() <= MAX_MODS,
        "too many saved mod selections"
    );
    Ok(preferences)
}
fn save_preferences(path: &Path, preferences: &Preferences) -> anyhow::Result<()> {
    write_atomic(path, toml::to_string_pretty(preferences)?.as_bytes())
}
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("mod data file has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".void-write-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "void-mod-test-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn manifest(id: &str, deferrable: bool) -> ModManifest {
        ModManifest::parse(&format!("id='{id}'\nversion='1.0.0'\nsdk=1\nruntime='wasm'\nentry='test.wasm'\ncapabilities=['hud']\ndeferrable={deferrable}")).unwrap()
    }
    fn add(host: &mut ModHost, id: &str, deferrable: bool, fail: bool) -> Arc<AtomicUsize> {
        let calls = Arc::new(AtomicUsize::new(0));
        let manifest = manifest(id, deferrable);
        host.available.push(Available {
            manifest: manifest.clone(),
            directory: host.directory.join(id),
            trust_native: false,
        });
        host.loaded.push(Loaded {
            manifest,
            instance: Instance::Test {
                calls: calls.clone(),
                fail,
            },
            hud: Vec::new(),
            error: None,
            enabled: true,
            key: JobKey(host.next_key),
            revision: 0,
            estimated_cpu_us: 150,
            fingerprint: None,
        });
        host.next_key += 1;
        calls
    }
    fn scheduler() -> Scheduler {
        Scheduler::new(void_core::EventHorizonConfig::default(), 240).unwrap()
    }
    fn admit(scheduler: &mut Scheduler) -> Vec<void_core::scheduler::AdmittedJob> {
        scheduler.admit(void_core::FrameBudget {
            now_us: 100,
            cpu_us: 10_000,
            gpu_us: 10_000,
        })
    }
    #[test]
    fn disabled_admitted_work_never_executes_and_reenable_preserves_other_ids() {
        let temp = Temp::new();
        let mut host = ModHost::new(temp.0.clone());
        let first = add(&mut host, "first", true, false);
        let second = add(&mut host, "second", true, false);
        let mut scheduler = scheduler();
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 0);
        let jobs = admit(&mut scheduler);
        assert_eq!(jobs.len(), 2);
        host.disable("first");
        for job in jobs {
            host.execute(job, FrameMetrics::default(), &mut scheduler, 100);
        }
        assert_eq!(first.load(Ordering::Relaxed), 0);
        assert_eq!(second.load(Ordering::Relaxed), 1);
        assert_eq!(scheduler.resident_bytes(), 0);
        assert_eq!(host.hud().count(), 1);
        host.enable(0);
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 200);
        for job in admit(&mut scheduler) {
            host.execute(job, FrameMetrics::default(), &mut scheduler, 300);
        }
        assert_eq!(first.load(Ordering::Relaxed), 1);
        assert_eq!(second.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn fault_isolated_and_failed_callback_does_not_count_as_publication() {
        let temp = Temp::new();
        let mut host = ModHost::new(temp.0.clone());
        let failed = add(&mut host, "failed", true, true);
        let good = add(&mut host, "good", false, false);
        let mut scheduler = scheduler();
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 0);
        for job in admit(&mut scheduler) {
            host.execute(job, FrameMetrics::default(), &mut scheduler, 100);
        }
        assert_eq!(scheduler.stats().published, 0);
        assert_eq!(scheduler.resident_bytes(), 0);
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 200);
        assert_eq!(failed.load(Ordering::Relaxed), 1);
        assert_eq!(good.load(Ordering::Relaxed), 2);
        assert!(!host.enabled("failed"));
        assert!(host.error("failed").is_some());
        assert!(
            !read_preferences(&temp.0.join("enabled.toml"))
                .unwrap()
                .modules["failed"]
                .enabled
        );
    }
    #[test]
    fn world_reset_discards_old_epoch_without_invoking_callback() {
        let temp = Temp::new();
        let mut host = ModHost::new(temp.0.clone());
        let calls = add(&mut host, "example", true, false);
        let mut scheduler = scheduler();
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 0);
        let job = admit(&mut scheduler).remove(0);
        scheduler.set_epoch(void_core::WorldRevision {
            world: 1,
            resources: 0,
        });
        host.reset_jobs();
        host.execute(job, FrameMetrics::default(), &mut scheduler, 100);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(scheduler.resident_bytes(), 0);
    }
    #[test]
    fn native_trust_binds_both_binary_and_manifest() {
        let mut manifest = manifest("native", false);
        manifest.runtime = Runtime::Native;
        let original = native_fingerprint(&manifest, b"original").unwrap();
        assert!(authorize_native(&NativePermission::None, &original).is_err());
        assert!(authorize_native(&NativePermission::Saved(original.clone()), &original).is_ok());
        assert!(
            authorize_native(
                &NativePermission::Saved(original.clone()),
                &native_fingerprint(&manifest, b"changed").unwrap()
            )
            .is_err()
        );
        manifest.version = "2.0.0".into();
        assert!(
            authorize_native(
                &NativePermission::Saved(original),
                &native_fingerprint(&manifest, b"original").unwrap()
            )
            .is_err()
        );
    }
    #[test]
    fn older_admitted_job_does_not_remove_newer_queue_marker() {
        let temp = Temp::new();
        let mut host = ModHost::new(temp.0.clone());
        let calls = add(&mut host, "example", true, false);
        let mut scheduler = scheduler();
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 0);
        let old = admit(&mut scheduler).remove(0);
        host.disable("example");
        host.enable(0);
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 200);
        let new_revision = host.loaded[0].revision;
        host.execute(old, FrameMetrics::default(), &mut scheduler, 300);
        assert_eq!(host.queued.get(&host.loaded[0].key), Some(&new_revision));
        host.prepare_frame(FrameMetrics::default(), &mut scheduler, 400);
        assert_eq!(host.loaded[0].revision, new_revision);
        for job in admit(&mut scheduler) {
            host.execute(job, FrameMetrics::default(), &mut scheduler, 500);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(scheduler.resident_bytes(), 0);
    }
    #[test]
    fn preferences_replace_atomically_and_invalid_file_is_not_overwritten() {
        let temp = Temp::new();
        let path = temp.0.join("enabled.toml");
        let mut preferences = Preferences::default();
        preferences.modules.insert(
            "example".into(),
            Selection {
                enabled: true,
                trusted_native_sha256: None,
            },
        );
        save_preferences(&path, &preferences).unwrap();
        preferences.modules.get_mut("example").unwrap().enabled = false;
        save_preferences(&path, &preferences).unwrap();
        assert!(!read_preferences(&path).unwrap().modules["example"].enabled);
        std::fs::write(&path, "not valid TOML {{{").unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut host = ModHost::new(temp.0.clone());
        host.disable("example");
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
