# Rust extension SDK v1

`void-sdk` is shared by built-in metrics, trusted native extensions, and sandboxed
Rust-to-Wasm modules. The implemented interface currently covers frame metrics and
HUD text. World queries, interaction requests, custom channels, keybind registration,
render hooks and dependency resolution are future compatibility work; manifests
requesting unsupported capabilities fail. The in-client screen scans modules,
enables/disables callbacks, remembers selections, shows faults and requires explicit
trust for native code. Approved native binaries are fingerprinted with their manifest;
changed code requires fresh trust and native replacement requires a restart.

## Build a portable Rust mod

```powershell
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown --manifest-path examples/mods/metrics-wasm/Cargo.toml
```

Copy `void_metrics_wasm.wasm` from the example's target directory beside its
`mod.toml`, then place that folder in the client's mods directory. Each mod owns one
folder and a relative entry path. Symlink escapes and parent paths are rejected.

The guest exports `void_abi_version() -> i32`, `void_on_frame()`, and linear
`memory`. Imports use the `void` namespace: `metric(index) -> f32` and
`hud_text(x, y, rgba, pointer, length) -> i32`. `void-sdk::wasm::Host` wraps this
wire interface. The host copies text before returning and accepts at most 1 KiB per
command and 256 commands per frame. Metric indices are FPS, ping milliseconds,
left CPS, right CPS, and the pressed-key bitmask. Packed color is `0xRRGGBBAA`.

Required manifest fields are `id`, semantic `version`, `sdk = 1`, `runtime`, and
`entry`. Optional `capabilities = ["hud", "metrics"]` enable the corresponding host
calls. Unknown capabilities are rejected. `settings` holds arbitrary TOML settings;
`deferrable` describes scheduler eligibility and does not itself schedule a callback.
`dependencies` is reserved and must currently be empty.

The default sandbox permits one memory up to 32 MiB, one instance/table, 4,096
table entries, a 512 KiB Wasm stack, 500,000 fuel per callback, and 1,024 host calls.
There are no WASI, file, network, clock, or thread imports. Instantiation/start code
also receives fuel. Permission violations, fuel exhaustion, growth failure, and
invalid text/pointers disable the offending module and discard its partial frame.
The module file limit is 32 MiB; Wasmtime/compiler allocations are separate from
the guest linear-memory limit. Compile modules on a worker before entering play.

## Trusted native mods

```powershell
cargo build --release --manifest-path examples/mods/metrics-native/Cargo.toml
```

Use the resulting DLL on Windows or SO on Linux. Adjust the manifest entry to the
platform filename. Native libraries export `void_mod_v1` returning a static
`ModApiV1`. Both tables carry an ABI version and byte size; function pointers,
scalars, and borrowed byte buffers are the only values crossing this boundary.
Never retain host pointers, pass Rust objects across the ABI, spawn callbacks that
outlive a frame, or unwind through C. Rebuild against SDK v1 for each platform.

The loader's `unsafe load_trusted` is an explicit caller trust boundary. Library
initializers run before ABI inspection. Native libraries can access the entire
process, crash it, or ignore budgets; permissions document host API access only.
They require restart to change, and their callbacks cannot be forcibly interrupted.

## Host integration

Read a `ModManifest`, load it with `WasmMod::load(directory, manifest, limits)`, and
invoke `on_frame(FrameMetrics)` for a vector of bounded HUD commands. Keep mod
errors separate from client errors; display `fault()` without retrying disabled
modules. Only callbacks explicitly marked deferrable can enter Event Horizon's
background budget. Frame metrics and required HUD state run every render opportunity.

Sources: [Wasmtime fuel and interruption](https://docs.wasmtime.dev/examples-interrupting-wasm.html),
[Wasmtime resource limits](https://docs.wasmtime.dev/api/wasmtime/struct.StoreLimitsBuilder.html).
