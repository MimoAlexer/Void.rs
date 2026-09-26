# Void.rs

Implement the approved standalone Minecraft Java 26.2 Rust client. Preserve vanilla gameplay and protocol behavior. Never present a stub or an offline scene as working multiplayer.

Use Bevy ECS and direct ash Vulkan. No Azalea dependency. TOML owns user settings, themes, layout and mod manifests. Keep networking and blocking asset work off the UI/render thread. Rust is pinned in rust-toolchain.toml.

Run meaningful unit/integration tests, cargo fmt --check and strict Clippy. Record platform and hardware verification separately from untested claims. Event Horizon stays experimental until real A/B measurements satisfy the documented gates.

Keep API seams between protocol, core, services, renderer and client. Do not put account tokens or signing keys into source, TOML, logs or command-line arguments. Do not commit Mojang assets or server jars.

Update docs/STATUS.md with concrete verified capabilities and outstanding compatibility work before publishing. Use parallel agents for independent crates where useful.
