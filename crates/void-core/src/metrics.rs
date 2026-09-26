//! Summaries from supplied measurements and explicit A/B acceptance gates.
//! No synthetic workload can enable Event Horizon through these APIs.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
pub struct FrameSample {
    pub frame_us: u64,
    /// None means no input event was represented by this frame. Missing events
    /// must not be counted as zero-latency measurements.
    pub input_to_submit_us: Option<u64>,
    /// None means no nearby update was presented during this frame.
    pub nearby_update_age_us: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    Rendered,
    Synthetic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummary {
    pub source: MeasurementSource,
    pub elapsed_us: u64,
    pub frame_count: usize,
    pub input_sample_count: usize,
    pub nearby_update_count: usize,
    pub average_fps: f64,
    pub p99_frame_us: f64,
    pub p95_input_to_submit_us: Option<f64>,
    pub p95_nearby_update_age_us: Option<f64>,
    pub maximum_frame_us: u64,
    pub visual_correct: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MetricsError {
    #[error("measurements require nonempty samples and nonzero elapsed time")]
    Empty,
    #[error("frame durations must be nonzero")]
    ZeroFrameDuration,
}

impl RunSummary {
    pub fn from_samples(
        samples: &[FrameSample],
        elapsed_us: u64,
        source: MeasurementSource,
        visual_correct: bool,
    ) -> Result<Self, MetricsError> {
        if samples.is_empty() || elapsed_us == 0 {
            return Err(MetricsError::Empty);
        }
        if samples.iter().any(|sample| sample.frame_us == 0) {
            return Err(MetricsError::ZeroFrameDuration);
        }
        let mut frames: Vec<_> = samples.iter().map(|s| s.frame_us).collect();
        let mut input: Vec<_> = samples
            .iter()
            .filter_map(|s| s.input_to_submit_us)
            .collect();
        let mut nearby: Vec<_> = samples
            .iter()
            .filter_map(|s| s.nearby_update_age_us)
            .collect();
        frames.sort_unstable();
        input.sort_unstable();
        nearby.sort_unstable();
        Ok(Self {
            source,
            elapsed_us,
            frame_count: samples.len(),
            input_sample_count: input.len(),
            nearby_update_count: nearby.len(),
            average_fps: samples.len() as f64 * 1_000_000.0 / elapsed_us as f64,
            p99_frame_us: percentile(&frames, 99) as f64,
            p95_input_to_submit_us: (!input.is_empty()).then(|| percentile(&input, 95) as f64),
            p95_nearby_update_age_us: (!nearby.is_empty()).then(|| percentile(&nearby, 95) as f64),
            maximum_frame_us: *frames.last().expect("nonempty samples"),
            visual_correct,
        })
    }

    fn valid(&self) -> bool {
        self.elapsed_us >= 120_000_000
            && self.frame_count >= 100
            && self.input_sample_count > 0
            && self.input_sample_count <= self.frame_count
            && self.nearby_update_count > 0
            && self.nearby_update_count <= self.frame_count
            && self.average_fps.is_finite()
            && self.average_fps > 0.0
            && self.p99_frame_us.is_finite()
            && self.p99_frame_us > 0.0
            && self
                .p95_input_to_submit_us
                .is_some_and(|v| v.is_finite() && v > 0.0)
            && self
                .p95_nearby_update_age_us
                .is_some_and(|v| v.is_finite() && v >= 0.0)
            && self.maximum_frame_us as f64 >= self.p99_frame_us
            && (self.average_fps - self.frame_count as f64 * 1_000_000.0 / self.elapsed_us as f64)
                .abs()
                <= self.average_fps * 0.001
    }
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    // Nearest-rank quantile: tail samples remain represented even for small sets.
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkEvidence {
    /// Must name hardware, resolution, world, settings, and build/mode.
    pub label: String,
    /// Identical scene/settings, fixed 240 Hz target, warm-up excluded.
    pub stressed: Vec<RunSummary>,
    /// Same scene/settings, uncapped rendering, warm-up excluded.
    pub uncapped: Vec<RunSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Acceptance {
    pub passed: bool,
    pub reasons: Vec<String>,
    pub p99_improvement_percent: Option<f64>,
    pub input_improvement_percent: Option<f64>,
}

/// Input represents actual measurements, never a performance prediction.
/// Median run results reduce a single-run outlier's influence. Visual correctness
/// and the absolute 240 FPS capacity requirement apply to every submitted run.
pub fn compare_runs(baseline: &BenchmarkEvidence, candidate: &BenchmarkEvidence) -> Acceptance {
    let mut result = Acceptance {
        passed: false,
        reasons: Vec::new(),
        p99_improvement_percent: None,
        input_improvement_percent: None,
    };
    for (name, evidence) in [("baseline", baseline), ("candidate", candidate)] {
        if evidence.label.trim().is_empty() {
            result
                .reasons
                .push(format!("{name}: measurement conditions need a label"));
        }
        if evidence.stressed.len() < 3 || evidence.uncapped.len() < 3 {
            result.reasons.push(format!(
                "{name}: requires three stressed and three uncapped runs"
            ));
        }
        for (index, run) in evidence
            .stressed
            .iter()
            .chain(evidence.uncapped.iter())
            .enumerate()
        {
            if run.source != MeasurementSource::Rendered {
                result.reasons.push(format!(
                    "{name} run {index}: synthetic measurements are not release evidence"
                ));
            }
            if !run.valid() {
                result.reasons.push(format!("{name} run {index}: incomplete/invalid measurements or duration below 120 seconds"));
            }
            if !run.visual_correct {
                result
                    .reasons
                    .push(format!("{name} run {index}: visual correctness failed"));
            }
        }
    }
    if !result.reasons.is_empty() {
        return result;
    }
    let base_frame = median(baseline.stressed.iter().map(|r| r.p99_frame_us));
    let next_frame = median(candidate.stressed.iter().map(|r| r.p99_frame_us));
    let base_input = median(
        baseline
            .stressed
            .iter()
            .map(|r| r.p95_input_to_submit_us.unwrap_or_default()),
    );
    let next_input = median(
        candidate
            .stressed
            .iter()
            .map(|r| r.p95_input_to_submit_us.unwrap_or_default()),
    );
    result.p99_improvement_percent = Some((1.0 - next_frame / base_frame) * 100.0);
    result.input_improvement_percent = Some((1.0 - next_input / base_input) * 100.0);
    if next_frame > base_frame * 0.8 && next_input > base_input * 0.85 {
        result.reasons.push(
            "neither 20% p99 frame-time nor 15% p95 input-to-submit improvement reached".into(),
        );
    }
    if next_frame > base_frame * 1.05 {
        result
            .reasons
            .push("p99 frame time regressed over 5%".into());
    }
    if next_input > base_input * 1.05 {
        result
            .reasons
            .push("p95 input-to-submit time regressed over 5%".into());
    }
    if candidate.uncapped.iter().any(|run| run.average_fps < 240.0) {
        result
            .reasons
            .push("uncapped average FPS fell below 240 in at least one run".into());
    }
    let base_fps = median(baseline.uncapped.iter().map(|r| r.average_fps));
    let next_fps = median(candidate.uncapped.iter().map(|r| r.average_fps));
    if next_fps < base_fps * 0.95 {
        result
            .reasons
            .push("uncapped median average FPS regressed over 5%".into());
    }
    let base_age = median(
        baseline
            .stressed
            .iter()
            .map(|r| r.p95_nearby_update_age_us.unwrap_or_default()),
    );
    let next_age = median(
        candidate
            .stressed
            .iter()
            .map(|r| r.p95_nearby_update_age_us.unwrap_or_default()),
    );
    if next_age > base_age * 1.05 {
        result
            .reasons
            .push("nearby terrain p95 presentation age regressed over 5%".into());
    }
    result.passed = result.reasons.is_empty();
    result
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    if values.len().is_multiple_of(2) {
        (values[values.len() / 2 - 1] + values[values.len() / 2]) / 2.0
    } else {
        values[values.len() / 2]
    }
}

/// Integer monotonic-clock pacing helper. Wait before sampling the render camera;
/// action events retain their own original timestamps/orientations independently.
#[derive(Clone, Debug)]
pub struct FramePacer {
    period_us: u64,
    next_us: Option<u64>,
}
impl FramePacer {
    pub fn new(target_fps: u32) -> Self {
        Self {
            period_us: if target_fps == 0 {
                0
            } else {
                (1_000_000 / u64::from(target_fps)).max(1)
            },
            next_us: None,
        }
    }
    pub fn wait_us(&self, now_us: u64) -> u64 {
        self.next_us.map_or(0, |next| next.saturating_sub(now_us))
    }
    /// Called once a frame is about to begin. Missed deadlines are skipped; no
    /// catch-up burst is emitted after a stall or window occlusion.
    pub fn begin_frame(&mut self, now_us: u64) {
        if self.period_us == 0 {
            self.next_us = None;
            return;
        }
        let next = self.next_us.unwrap_or(now_us);
        self.next_us = Some(if next <= now_us {
            next.saturating_add(
                (now_us.saturating_sub(next) / self.period_us)
                    .saturating_add(1)
                    .saturating_mul(self.period_us),
            )
        } else {
            next
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence(frame: f64, input: f64, fps: f64) -> BenchmarkEvidence {
        let run = RunSummary {
            source: MeasurementSource::Rendered,
            elapsed_us: 120_000_000,
            frame_count: (fps * 120.0) as usize,
            input_sample_count: 100,
            nearby_update_count: 100,
            average_fps: fps,
            p99_frame_us: frame,
            p95_input_to_submit_us: Some(input),
            p95_nearby_update_age_us: Some(5000.0),
            maximum_frame_us: 20_000,
            visual_correct: true,
        };
        BenchmarkEvidence {
            label: "unit fixture, not release measurement".into(),
            stressed: vec![run.clone(); 3],
            uncapped: vec![run; 3],
        }
    }
    #[test]
    fn acceptance_thresholds_are_inclusive_and_independent() {
        let base = evidence(10_000.0, 5000.0, 300.0);
        assert!(compare_runs(&base, &evidence(8000.0, 5250.0, 285.0)).passed);
        assert!(!compare_runs(&base, &evidence(8000.1, 5000.0, 300.0)).passed);
        assert!(compare_runs(&base, &evidence(10_500.0, 4250.0, 300.0)).passed);
        assert!(!compare_runs(&base, &evidence(10_500.1, 4250.0, 300.0)).passed);
        assert!(!compare_runs(&base, &evidence(8000.0, 5250.1, 300.0)).passed);
        assert!(!compare_runs(&base, &evidence(8000.0, 5000.0, 284.0)).passed);
        assert!(
            !compare_runs(
                &evidence(10_000.0, 5000.0, 245.0),
                &evidence(8000.0, 5000.0, 239.0)
            )
            .passed
        );
    }
    #[test]
    fn synthetic_or_incomplete_evidence_cannot_pass() {
        let base = evidence(10_000.0, 5000.0, 300.0);
        let mut candidate = evidence(7000.0, 4000.0, 300.0);
        candidate.stressed[0].source = MeasurementSource::Synthetic;
        assert!(!compare_runs(&base, &candidate).passed);
        candidate.stressed[0].source = MeasurementSource::Rendered;
        candidate.stressed[1].p95_nearby_update_age_us = None;
        assert!(!compare_runs(&base, &candidate).passed);
        candidate.stressed[1].p95_nearby_update_age_us = Some(5000.0);
        candidate.stressed[2].visual_correct = false;
        assert!(!compare_runs(&base, &candidate).passed);
        candidate.stressed.clear();
        assert!(!compare_runs(&base, &candidate).passed);
    }
    #[test]
    fn percentile_uses_tail_and_real_elapsed_time() {
        let frames: Vec<_> = (1..=100)
            .map(|i| FrameSample {
                frame_us: i,
                input_to_submit_us: Some(i),
                nearby_update_age_us: None,
            })
            .collect();
        let summary =
            RunSummary::from_samples(&frames, 1_000_000, MeasurementSource::Synthetic, false)
                .unwrap();
        assert_eq!(summary.p99_frame_us, 99.0);
        assert_eq!(summary.p95_input_to_submit_us, Some(95.0));
        assert_eq!(summary.average_fps, 100.0);
        assert_eq!(summary.p95_nearby_update_age_us, None);
        assert!(RunSummary::from_samples(&[], 1, MeasurementSource::Rendered, true).is_err());
    }
    #[test]
    fn pacer_skips_missed_frames_and_supports_uncapped() {
        let mut pacer = FramePacer::new(250);
        pacer.begin_frame(0);
        assert_eq!(pacer.wait_us(1000), 3000);
        pacer.begin_frame(100_000);
        assert_eq!(pacer.wait_us(100_000), 4000);
        let mut uncapped = FramePacer::new(0);
        uncapped.begin_frame(10);
        assert_eq!(uncapped.wait_us(10), 0);
    }

    #[test]
    fn frames_without_input_cannot_dilute_latency_measurements() {
        let mut frames = vec![
            FrameSample {
                frame_us: 4000,
                input_to_submit_us: None,
                nearby_update_age_us: None
            };
            100
        ];
        frames[0].input_to_submit_us = Some(8000);
        let summary =
            RunSummary::from_samples(&frames, 400_000, MeasurementSource::Synthetic, false)
                .unwrap();
        assert_eq!(summary.input_sample_count, 1);
        assert_eq!(summary.p95_input_to_submit_us, Some(8000.0));
        frames[0].input_to_submit_us = None;
        let summary =
            RunSummary::from_samples(&frames, 400_000, MeasurementSource::Synthetic, false)
                .unwrap();
        assert_eq!(summary.p95_input_to_submit_us, None);
    }
}
