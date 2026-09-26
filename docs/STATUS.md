# Implementation status

This is an incomplete development implementation of the approved plan. It is not ready to replace a vanilla PvP client.

| Area | Implemented | Remaining |
| --- | --- | --- |
| Project | Cargo workspace, pinned Rust, MIT, modular crates | CI platform outcomes and ongoing releases |
| Vulkan | Native window, GLSL/SPIR-V, direct ash, UI/terrain pipelines, two-frame fencing, GPU timestamps, capture | Validation-layer audit, resource lifetime hardening, Linux GPU testing, granular uploads/culling/batching |
| UI | Black-hole shader fallback, menus, settings, server status, account action, diagnostics | Complete TOML layouts, MP4 playback, full vanilla screens/accessibility |
| Configuration | Typed TOML, migration, atomic save, last-valid reload, metadata/HUD/keybindings | Wire every exposed setting to behavior; profile editor |
| Protocol | Independent776 codec, compression, login/configuration/play subset, keepalive, teleport, block/entity events | All missing packets and associated semantics, signed chat, inventory transactions, resource-pack lifecycle, transfers |
| Online | Browser PKCE, Xbox/Minecraft exchange, entitlement/session checks, OS credentials, RSA/AES-CFB8/session join, GUI saved accounts/refresh/switching | End-to-end registered-account verification, signed chat and complete account lifecycle UI |
| Terrain | Official state classification/collision data, asynchronous diagnostic geometry | Original models/textures/light/biomes/translucency/block entities and complete resource packs |
| Movement | Basic dry-land walking, sprint acceleration, jumping, swept shapes | Steps, swimming, climbing, vehicles, effects, all attributes and differential validation |
| Entities/combat | Network state/ECS, diagnostic entity boxes; swing/attack packets, standing-player raycast with 3-block reach and terrain occlusion | Complete models/animation, all entity poses/attributes, cooldown/shield/effect behavior and inventory |
| Event Horizon | Bounded admission, deadlines/fairness, revisions, CPU/GPU estimates, evidence gates, limited app integration | Full frame coordination, granular upload cost, reproducible PvP A/B and Sodium comparison; default remains baseline |
| Mods | Real Wasmtime and trusted native loader; HUD/metrics SDK, compiled samples, saved enable/disable and native trust fingerprints | Complete deep SDK, dependency resolution, all built-in PvP features, polished management |
| Assets | Verified metadata/index, import/cache/download APIs | Full corpus/render integration and pack handling |
| Launcher | Local launch, setup/accounts, signed install, stage/rollback, running-client lock | Publisher registration/key provisioning, published signed release and real platform install qualification |

## Evidence

Per-crate verification has passed for core, protocol, SDK/services and launcher. See the final validation report for workspace-level commands and actual framebuffer/network checks. Compiled Wasm and native Rust examples were loaded successfully. Mojang26.2 metadata and asset index32 (5,057 objects) were verified live. This is not evidence of a complete downloaded/rendered asset set.

The complete compatibility checklist remains a release blocker. No240FPS PvP claim or Event Horizon improvement claim has been established.
