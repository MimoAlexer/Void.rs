# Approved implementation plan

## Product

Void.rs is to become a complete standalone Minecraft Java26.2 multiplayer client for Windows/Linux x86-64. Use Rust, Bevy ECS, direct Vulkan1.4 through ash, GLSL/SPIR-V, and an independent protocol implementation. Preserve vanilla behavior and provide its render-distance range. Own code is MIT, hosted at MimoAlexer/Void.rs.

Full multiplayer means all connection states, authentication, game data, movement/combat, interactions/inventories/crafting, entities, dimensions, chat/signing/commands, server-driven screens, resource packs, audio, localization and accessibility. Complete vanilla rendering is required for release.

Not in the first complete release: singleplayer simulation, Bedrock, Java mod execution, OptiFine extensions, existing shader packs, gameplay replay, online mod marketplace.

## Event Horizon

Develop a measured input/frame/background-work coordinator for sword/axe/shield PvP. Preserve mandatory gameplay ordering. Bound queued work and extra CPU cache memory (default256MiB), use measured independent CPU/GPU costs, maintain visual freshness and background fairness, and discard obsolete work by revision.

Baseline, frame-coordination, and full-governor variants must be compared with identical rendering settings. Do not enable experimentally better scheduling by default until real stressed and uncapped workloads meet the gates in EVENT_HORIZON.md. Target240FPS at1080p/12chunks, p99 steady frame time<=8.33ms, streaming p99<=16.67ms on the documented RTX5060 laptop. Maximum render distance is a separate stability test.

## User interface, assets, accounts

Black-hole menus; future user MP4 provides skippable intro and silent menu loop, with static fallback and no gameplay decoding. TOML controls settings, profiles, menus/layouts, HUD, themes, manifests and exposed mod settings. Support migrations, atomic writes, useful errors, last-valid reload and restart labels.

Microsoft browser login uses secure credentials, refresh, account switching and entitlement/session verification. A separate offline identity supports offline-mode testing. Reuse compatible local assets, otherwise download version-matched original assets and verify hashes.

## Modding

Provide a Rust SDK and local loader, portable sandboxed Wasm and explicitly trusted native DLL/SO extensions. Use versioned host APIs and a C-compatible native boundary with no Rust-layout assumptions. Expose HUD/UI, input, events, queries, interactions, custom channels and render hooks. Implement the built-in PvP suite through SDK contracts: FPS/ping/CPS/keys, armor/effects, zoom, toggle sprint, crosshair, clutter controls and HUD editing.

## Delivery order

1. Foundation and authentication feasibility.
2. Playable independent-protocol baseline with real renderer.
3. Event Horizon instrumentation and controlled experiments.
4. Complete multiplayer and vanilla visual parity.
5. Full modding, black-hole interface, assets/accounts.
6. Signed launcher updates, packaging and release qualification.

Launcher stable/preview channels use signed manifests, verified staged artifacts and rollback; local launch works if update service is down. Launcher exits during gameplay and cannot replace a running installation.

## Release gates

Unit/fixture/fuzz tests, differential vanilla movement/combat tests, vanilla/Paper integrations, visual comparisons, mod-isolation failures, corrupt configurations, auth expiry, missing video, interrupted downloads/updates, Vulkan validation and long gameplay sessions. Windows/Linux build and hardware-backed runtime verification are distinct. Record unsupported features and external prerequisites rather than reducing the definition of complete.

External inputs: user video, registered Microsoft application and publisher signing credentials. No milestone or synthetic/menu benchmark independently satisfies the complete-client requirement.
