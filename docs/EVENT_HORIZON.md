# Event Horizon

Event Horizon is an **experimental scheduler**, not a proven performance advantage.
The default is `baseline`. Scheduler unit tests and synthetic workloads are not
evidence of Minecraft rendering speed, vanilla compatibility, or input-to-photon latency.

## Integration contract

`void_core::scheduler::Scheduler` admits explicitly deferrable jobs against
separate available CPU and GPU microsecond budgets. Compute those budgets after
required rendering, gameplay, and network processing. Submit bounded mesh/asset
upload batches and explicitly deferrable callbacks. Never enqueue simulation,
network updates, input actions, mandatory callbacks, or required HUD state.

- `baseline` uses ordinary visibility/distance priority and caller cost estimates.
- `frame_coordination` retains the same job policy; the frontend applies pacing
  waits before camera sampling and bounds GPU frames in flight.
- `event_horizon` additionally adapts per-kind estimates using rolling means and
  deviations, reserves spare capacity for aged work, prioritizes expired deadlines,
  and applies a configurable safety margin independently to both budgets.

All three modes retain necessary revision, memory, and queue correctness checks.
All modes need a competent asynchronous renderer. Changes to visual settings are
not part of this algorithm. Fairness reserves apply to fitting aged work; expired
larger work also gains priority in the main queue. No scheduler can guarantee a
deadline under sustained overload or execute a batch that exceeds every budget.
Split oversized jobs before enqueueing. Native callback overruns are reported;
they cannot be interrupted safely. Wasm limits belong to the runtime host.

Workers receive an `AdmittedJob` ticket. Report completion on the renderer/state
owner thread, then atomically replace the visible resource only for
`Completion::Publish`. `DiscardStale` drops the result. Per-key content revisions
coalesce queue entries without resetting their age. A world/resource epoch change
invalidates all older work, including already running jobs. A worker's memory is
charged until it actually completes or confirms cancellation. Never publish a
result before checking its ticket, and do not introduce an asynchronous gap
between that check and publication.

Call `invalidate_content(key, revision)` immediately when authoritative chunk or
resource content changes, before allocating/enqueueing replacement work. This
rejects old results even if replacement allocation or enqueueing fails under
backpressure. The previously published resource may remain visible until a valid
replacement exists; record its age separately.

Memory accounting charges each descriptor plus caller-declared retained payload
bytes. Callers must charge every extra retained buffer and release it on completion.
This is a bound on scheduler-accounted storage, not total renderer/world/process
memory. A maximum job count independently bounds descriptors and accounting maps.

`FramePacer` uses caller-supplied monotonic microseconds, skips missed presentation
deadlines, and supports uncapped presentation. Timestamped `InputAction`s retain
the exact orientation at action time, independently of later camera sampling.

## Configuration

See `config/default.toml`. `[performance].target_fps` is shared by pacing and
scheduling; zero disables the presentation cap and retains a 240 Hz scheduling
budget target. `[performance.event_horizon]` selects `mode`, 256 MiB extra memory,
4096 jobs, a two-frame nearby deadline, 250 ms aging, 10% fairness reserve,
150 microsecond budget margins, and at most two frames in flight by default.

The strict TOML schema rejects misspelled fields and unsafe values. Safe settings
can reload through `ConfigStore`; parse/validation errors leave the last valid
configuration intact. Graphics and scheduler/performance changes are marked as
requiring renderer restart. Consumers must apply reload flags appropriately.
Saves flush a temporary file and atomically replace the destination. Schema 0's
`graphics.frame_limit` migrates to `performance.target_fps` in memory; migration
does not rewrite the source until an explicit save. Future schema versions fail
with an actionable error.

## Reproducible evidence

Run the same deterministic duel scenes/settings in all three modes. Test busy
arenas, camera turns, chunk arrival bursts, GPU saturation, and controlled mod
load. Include nearby terrain updates so visual staleness is measurable. Record
CPU/GPU time, queue depth, memory, frame intervals, input-to-submit latency, and
update age. Run three 120-second trials after warm-up at 1080p/12 chunks with
vanilla textures, fast graphics, clouds off, HUD on, and VSync off. Repeat with a
240 FPS target and uncapped rendering. Record power profile, device/driver,
hardware, build, world, settings, and mode. Compare compatible pinned Sodium
separately; that comparison cannot establish the scheduler's individual benefit.

`RunSummary::from_samples` computes nearest-rank quantiles from measurements.
`BenchmarkEvidence` contains a descriptive `label`, three `stressed` summaries,
and three `uncapped` summaries. Each run records `source` (`rendered` or
`synthetic`), elapsed time, frame/input/update counts, FPS, p99 frame time, p95
input-to-submit time, p95 nearby-update age, maximum frame time, and visual
correctness. Labeling data `rendered` is an assertion by the measurement producer,
not independent attestation. Do not alter reports to make a gate pass.
Frames without input use `None`, not zero latency, and are excluded from input
quantiles; a run without observed input cannot pass the responsiveness gate.

Compare actual reports with:

```text
cargo run -p void-core --example compare_metrics -- baseline.json event-horizon.json
```

This exits 0 only if all gates pass, 2 for failed acceptance, and 1 for invalid
input. It does not change the user's selected mode. Release acceptance requires:

- At least 20% p99 frame-time improvement **or** 15% p95 input-to-submit improvement.
- Neither responsiveness metric regresses by over 5%.
- Every candidate uncapped run averages at least 240 FPS, with at most 5%
  regression in median uncapped FPS versus baseline.
- Visual correctness passes every run and median p95 nearby-update age regresses
  by no more than 5% (the concrete definition of materially worse freshness).
- Both conditions contain at least three actual rendered 120-second trials,
  valid measurements, and observed nearby updates. Synthetic/incomplete evidence
  cannot pass. Comparisons use medians of run summaries to resist single-run noise.

Separate original release gates still apply: steady-state p99 <=8.33 ms,
streaming p99 <=16.67 ms, full compatibility, platform qualification, and actual
gameplay testing. Input-to-submit is not input-to-photon; the latter requires
external measurement. No benchmark result is supplied or claimed by this crate.

## Windows measurement tools

`scripts/measure.ps1` provides a **trial-plan generator and descriptive log
analyzer**, not an automatic 120-second benchmark runner. The client currently
exposes `--smoke-frames`; frame counts do not establish a timed measurement.

Prepare three repetitions of every mode at 240 FPS and uncapped, using the same
base configuration and an explicitly selected server:

```powershell
cargo build --release -p void-client
./scripts/measure.ps1 -Prepare -ServerAddress 'localhost:25565' -RunDirectory '.local/duel-trials'
```

This writes 18 isolated TOML configurations, `run-plan.json`, and `commands.ps1`.
It launches nothing and changes no server settings/EULA. The configurations fix
1920x1080, 12 chunks, and VSync off; only mode and FPS cap differ across trials.
Mode order rotates between repetitions to reduce ordering bias. All configurations
share the run directory's `mods` folder; prepare the same controlled mod load
there. Existing user mods are not copied silently.

Review the generated commands and run trials interactively. Each command uses:

```powershell
./target/release/void-client.exe --config '.local/duel-trials/paced240-baseline-1.toml' --connect 'localhost:25565' --metrics '.local/duel-trials/paced240-baseline-1.json'
./target/release/void-client.exe --config '.local/duel-trials/paced240-frame_coordination-1.toml' --connect 'localhost:25565' --metrics '.local/duel-trials/paced240-frame_coordination-1.json'
./target/release/void-client.exe --config '.local/duel-trials/paced240-event_horizon-1.toml' --connect 'localhost:25565' --metrics '.local/duel-trials/paced240-event_horizon-1.json'
```

These processes run until their windows are closed. Use an external stopwatch:
allow the same scene to load, warm up for at least 15 seconds, perform the fixed
workload for at least 120 seconds, then close normally so metrics are saved. Record
the actual warm-up duration from startup through scene readiness; pass that value
to the analyzer. The analyzer discards complete boundary frames when trimming,
so capture a few extra seconds beyond the requested duration. Online-mode servers
require signing in through the client; a test username is not online authentication.

Analyze captured frame logs from a PowerShell session:

```powershell
./scripts/measure.ps1 -Metrics @('.local/duel-trials/paced240-baseline-1.json', '.local/duel-trials/paced240-frame_coordination-1.json', '.local/duel-trials/paced240-event_horizon-1.json') -WarmupSeconds 30 -DurationSeconds 120 -Output '.local/duel-trials/analysis.json'
```

The report includes the duration actually present, FPS calculated as frame count
divided by summed intervals, nearest-rank p99/p99.9/max frame time, optional-input
p95, and CPU/GPU/fence-wait quantiles. No-input frames contribute no latency
sample. Too-short logs, absent input, missing terrain, and retention/cap limits are
reported explicitly. A successful script exit means analysis succeeded; it does
not mean performance acceptance passed. The raw schema lacks nearby-update age
and independent visual validation, so every report remains `not_evaluable` for
release acceptance and makes no speedup claim.

Before evaluating an actual performance claim, record all of the following:

- Same release build, server/world, start position, camera/action sequence,
  server load, resource settings, mod load, resolution, and power profile.
- Three complete warm-up-excluded 120-second measurements per condition; no
  disconnects, menu intervals, missing terrain, truncated logs, or capture stalls.
- Real input activity, supported GPU timestamps, nearby-update-age measurements,
  and a separately checked visual-correctness result.
- Separate steady-state and streaming scenarios and the documented absolute FPS,
  tail-latency, regression, and freshness gates.

`./scripts/test-measure.ps1` validates the analyzer/plan contracts using explicitly
synthetic fixtures. Those tests launch no client and provide no performance evidence.
