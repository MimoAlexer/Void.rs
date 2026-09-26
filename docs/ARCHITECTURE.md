# Architecture

| Crate | Responsibility |
| --- | --- |
| void-client | winit event loop, egui interface, input, network-event application, background meshing, integrated scheduler |
| void-render | ash Vulkan instance/device/swapchain, two frame slots, terrain and UI pipelines, timestamp queries, framebuffer capture |
| void-core | typed configuration, Bevy ECS state, ordered input, restricted physics, Event Horizon, evidence gates |
| void-protocol | independent 26.2 codec, status, login/configuration/play subset, encryption, generated registries/collision shapes |
| void-sdk | Rust HUD API, versioned C interface, Wasm imports |
| void-services | asset verification/cache, Microsoft login, credentials, native/Wasm runtimes, signed installation/rollback |
| void-launcher | local launch, account/asset setup, channel manifests, staged updates and rollback |

## Threads and ownership

The window/render thread owns Vulkan and UI state. A dedicated network worker emits ordered events through a bounded channel. Worker threads receive immutable terrain snapshots and return geometry. Account login, downloads and module compilation run outside the render loop. The renderer fences per-frame buffers and uses per-swapchain-image presentation semaphores.

Bevy ECS stores authoritative entity snapshots. Restricted player prediction runs at a nominal20Hz; it does not imply complete vanilla physics. Server corrections override predictions. The protocol worker independently services keepalives, tick-end and connection state transitions.

TOML parses into typed state. Invalid reloads retain the previous configuration. Shader programs and executable behavior remain code; original Minecraft asset formats remain unchanged.

## Current integration limits

Terrain initially uses a whole visible-world mesh assembled on a worker, with bounded output and two frame-local buffers. This is a correctness/debugging baseline, not the planned efficient per-section renderer. It may allocate, rebuild or upload more than the final engine should. Models, textures, light, translucency and complete entity rendering are outstanding.

Event Horizon has real admission, cost-history and revision logic. Coordinated modes poll frame-slot fences without blocking the event loop so input can continue arriving under GPU backpressure. Baseline uses conventional blocking fence preparation. Fixed two-frame queue control is common to all modes, and swapchain acquisition still blocks. Granular GPU-upload accounting and presentation timing need further integration. Do not describe mode differences as validated performance improvements.

The mod SDK currently implements HUD and metrics. Unsupported world/interaction/network/render capabilities fail explicitly. Native code is trusted process code. Wasm execution has fuel, memory, stack, table, module-size and host-call limits and no ambient WASI access.

The launcher and client share an installation lock so updates cannot replace a running installed client. Manifests are verified against an explicitly configured publisher key; no fallback accepts unsigned updates.
