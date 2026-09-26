# Development validation — 2026-09-26

This report describes the initial development implementation. **The complete-client release gate has not passed.**

## Local environment

Windows x86-64, Intel Core i7-14700HX, NVIDIA GeForce RTX 5060 Laptop GPU, Vulkan 1.4.341, Rust 1.98.1. Framebuffer tests used 1920 × 1080, configured distance 12 and VSync off. The Khronos validation layer was absent; these runs do not count as validation-layer verification. Linux GPU execution remains unverified.

## Build and deterministic checks

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked` | 71 tests passed; none failed or ignored |
| `cargo build --release --locked -p void-client -p void-launcher` | Pass |

The tests include bounded and compressed protocol sessions, NIST AES-CFB8 data, PKCE callback validation, configuration recovery, scheduler invalidation/fairness, dry-land collision prediction, target reach/occlusion, native-trust fingerprints, Wasm fuel/memory traps, and signed-update rollback/tamper rejection. They do not establish complete vanilla mechanics.

Rust native and Wasm example mods were compiled and loaded through the real runtime probes, producing four HUD commands each. Mojang 26.2 metadata and asset index 32 (5,057 objects) were verified; the full asset corpus was not downloaded or rendered.

## Actual application smoke tests

The local `fixture_server` example sends a compressed offline login/configuration sequence, nine chunks, player position, a moving player entity, health and keepalives. It is a deterministic protocol fixture, **not a vanilla or Paper server**. No server EULA was accepted or server world started.

| Run | Submitted frames | Received chunks | Terrain vertices | GPU timestamp samples |
| --- | ---: | ---: | ---: | ---: |
| Menu, baseline | 180 | 0 | 0 | 178 |
| Fixture terrain, baseline | 1,000 | 9 | 30,564 | 998 |
| Fixture terrain, frame coordination | 600 | 9 | 30,564 | 598 |
| Fixture terrain, Event Horizon | 600 | 9 | 30,564 | 598 |
| Launcher → client menu | 120 | 0 | 0 | 118 |

Every client run exited successfully. Two initial GPU timestamp samples are absent because measurements are collected after frame-slot completion. The fixture logged teleport acknowledgement and completion of initial loading. Framebuffer PNGs were visually inspected: [menu](images/menu.png), [terrain and diagnostic player](images/terrain.png).

These are short smoke tests with minimal geometry and no controlled input stream. They are **not performance acceptance runs**, do not prove the 240 FPS gameplay target, and cannot establish an Event Horizon speedup. Screenshot readback deliberately stalls the final captured frame, so captured runs must not be used for frame-time qualification. No input-to-photon measurement was performed.

Example reproduction (separate terminals):

```sh
cargo run -p void-protocol --example fixture_server -- 127.0.0.1:25566
target/release/void-client --status 127.0.0.1:25566
target/release/void-client --config .local/smoke.toml --connect 127.0.0.1:25566 --smoke-frames 1000 --screenshot .local/terrain.png --metrics .local/frames.json
```

Use `.exe` on Windows. Stop the fixture before rebuilding its executable on Windows. Screenshots intentionally suppress the connection overlay during `--smoke-frames`; ordinary gameplay can dismiss it using **Look around**.

## Initial Windows release-build hashes

| Executable | SHA-256 |
| --- | --- |
| `void-client.exe` | `cbfc085fbdf45c290ca38a09043d16a8525b00082a00ae1dd4df4bfa746193ce` |
| `void-launcher.exe` | `726b60273060c121bde8a724ea2c3111d661822685c31c6117ecf7e2de131794` |

These hashes identify the locally tested development binaries; they are not a signed update manifest or publisher authentication.

## Open qualification gates

Microsoft registration and live licensed-account sign-in; real vanilla/Paper scenarios; inventories/crafting, signed chat, packs, full models/textures/lighting, audio, skins, localization/accessibility, remaining movement/combat mechanics; full mod SDK and PvP suite; MP4 worker; granular terrain scheduling; three matched 120-second PvP runs plus uncapped trials and freshness/correctness evidence; Vulkan validation and resize/stress recovery; Linux Wayland/X11 GPU tests; fuzz campaigns; signed platform distribution and recovery testing. See [implementation status](STATUS.md).

GitHub CI is configured for Windows and Ubuntu. Remote results are separate from these local results and must be checked before relying on them.
