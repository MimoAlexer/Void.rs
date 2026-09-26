# Minecraft Java 26.2 release compatibility checklist

**Release gate: incomplete.** Void.rs currently provides a development client and
an independently implemented protocol subset. It has not demonstrated complete
vanilla multiplayer support, competitive correctness, or the promised performance
targets. A menu, decoded world, diagnostic box, fixture connection, passing unit
test, or successful build does not close a gameplay compatibility requirement.

This checklist covers the approved Windows/Linux x86-64, Vulkan 1.4, Java **26.2**
release. Singleplayer simulation, Bedrock, existing Java mods, OptiFine/shader-pack
compatibility, gameplay replay and a hosted mod marketplace are outside that
release. Minecraft protocol version **776** and data version **4903** are pinned.

## Status and evidence rules

| Status | Meaning |
| --- | --- |
| **Implemented** | The stated narrow behavior exists and has local automated evidence. It does not imply its entire subsystem is finished. |
| **Partial** | A usable subset exists; behavior listed in the acceptance column remains incomplete. |
| **Unverified** | An implementation or infrastructure exists, but the required external/platform/gameplay evidence has not been obtained. |
| **Missing** | The required behavior is not implemented in this build. |

Every release acceptance row needs a recorded result identifying the client
commit/build, platform, reference/server version, test world/scenario, expected
and actual behavior, and an artifact such as logs, assertions or a visual capture.
An unchecked row remains a release blocker. Tests that need accounts, publisher
keys or an operator-run server remain unverified until those inputs are supplied
and the test is actually performed. Do not automatically accept a server EULA.

## Multiplayer, protocol and accounts

| Release check | Current status and evidence | Acceptance criteria |
| --- | --- | --- |
| [ ] Protocol version and generated data | **Implemented**: official jar metadata, packet report, block registry and golden codec fixtures. | Regenerate from the pinned official binaries, verify hashes and IDs, and fail clearly on a mismatched server version. |
| [ ] Server address entry, status and ping | **Implemented**: hostname/IP/port parsing and socket fixture status/echo tests. | Verify IPv4, IPv6, ordinary DNS, valid/invalid ports, MOTD/player counts and protocol mismatch against vanilla and Paper. |
| [ ] SRV discovery and LAN discovery | **Missing**. | Resolve Minecraft SRV targets with fallback; discover advertised LAN servers without blocking rendering. |
| [ ] Framing and compression | **Implemented**: bounded incremental codec, fragmented/compressed socket test. | Pass official fixtures and sustained fuzzing; malformed lengths, truncation and decompression bombs fail within configured bounds. |
| [ ] NBT and strings | **Implemented**: network root, depth/count limits and modified UTF-8 tests. | Decode reference registry/component data; fuzz every tag and nested collection; no panic, unbounded allocation or silent truncation. |
| [ ] Offline login/configuration | **Partial**: fixture-tested state transitions, keepalive/ping, settings, brand, known packs and registry subset. | Join an operator-run offline vanilla and Paper server, enter play, reconfigure, disconnect and reconnect without discarded required state. |
| [ ] Microsoft sign-in and account storage | **Unverified**: browser PKCE/provider exchange, entitlement checks, refresh and OS credential APIs exist. | Complete registered-app sign-in, denial/cancellation, expired/revoked credentials, account switching and logout; never expose secrets in logs/TOML/arguments. |
| [ ] Encrypted online server login | **Unverified**: RSA/AES-CFB8 and session join implemented; NIST/RSA tests pass. | Join real online-mode vanilla and Paper servers with a licensed account; verify refreshed-token success and correct failures for invalid entitlement/session. |
| [ ] Dynamic registries and datapacks | **Partial**: dimension bounds consumed; most registry data is not applied. | Use all received registries/tags, custom dimensions/biomes and datapack-defined content with version-correct lookup and cache invalidation. |
| [ ] Packet ordering and bundles | **Partial**: ordered application events and bounded queues; atomic bundle publication missing. | Apply all packet bundles atomically, including spawn/update/removal and dimension transitions; backpressure never silently corrupts state. |
| [ ] Transfers and cookies | **Missing**: transfer diagnosed as unsupported; cookie replies are empty. | Preserve permitted cookie state and complete server-directed transfers with correct destination/session handling and user-visible result. |
| [ ] Resource-pack negotiation | **Missing**: packs are explicitly declined. | Prompt/apply/remove optional and required packs; verify hashes, precedence, reload, refusal and failed download semantics without leaking URLs/tokens. |
| [ ] Server conduct and dialogs | **Partial**: conduct event requires explicit acceptance command; full server-driven dialogs are absent. | Display exact conduct/dialog content and controls; only accept after user action; handle disconnect/refusal and all vanilla server dialog types. |
| [ ] Disconnects, timeouts and recovery | **Partial**: errors and timeouts surfaced; broad impairment qualification absent. | Recover after refused connections, half-open sockets, network loss, server restart, malformed packets, compression transition and interrupted loading. |

## World rendering, assets and sound

| Release check | Current status and evidence | Acceptance criteria |
| --- | --- | --- |
| [ ] Asset acquisition and cache | **Partial**: verified metadata/index and import/download APIs; full corpus is not integrated. | Reuse a correct local installation; fetch only missing/corrupt version-matched assets; resume safely and prove render/audio consumers use the verified cache. |
| [ ] Terrain packet consumption | **Partial**: palettes/biomes decoded; block entities/light tails validated but not applied. | Correctly load/update/unload all sections, heightmaps, lights, biomes, block entities and batch acknowledgements in every dimension. |
| [ ] Block states and shapes | **Implemented**: 32,366 generated states, 326 static collision shapes and six physics-factor triples. | Reproduce tables; verify all air variants; add entity/world-dependent shapes and compare representative state transitions with vanilla. |
| [ ] Vanilla block and item models | **Missing**: colored diagnostic terrain is used. | Render every official block/item model, state variant, multipart geometry, atlas texture, transform and animation with reference parity. |
| [ ] Lighting, materials and transparency | **Missing**. | Compare sky/block light, ambient occlusion, emissive faces, cutouts, translucent ordering, fluids, tinting and pack-defined materials against vanilla. |
| [ ] Entity and block-entity visuals | **Partial**: generic diagnostic boxes and entity-position events. | Render every registered entity and block entity, metadata-driven pose/equipment/animation, player skins/capes and relevant effects. |
| [ ] Dimensions, terrain transitions and world border | **Partial**: dimension bounds/reset paths; complete visuals/rules absent. | Enter/leave Overworld, Nether, End and datapack dimensions; verify loading, respawn, fog/sky/lighting, border presentation and changed registries. |
| [ ] Weather, sky and particles | **Missing**. | Match day/night, weather, underwater/lava presentation and every particle type/parameter; obey vanilla options. |
| [ ] Sound and music | **Missing**. | Resolve/play/stop all registered sounds, spatial attenuation, categories, subtitles, music and resource-pack overrides without render-thread decoding. |
| [ ] Text, fonts and localization | **Partial**: application UI text; full vanilla component/font/language rendering absent. | Render translated/styled/NBT chat components, Unicode/bidirectional text, resource-pack fonts, key names and selectable languages correctly. |
| [ ] Render distance and GPU selection | **Partial**: 2–32 bounds and discrete-first Vulkan selection; full option behavior/qualification remains. | Exercise every supported distance, server limits and GPU choice; min/max settings must not drop geometry silently or claim 240 FPS at maximum distance. |
| [ ] Renderer lifetime and recovery | **Unverified**: synchronization/cleanup paths implemented; full GPU qualification pending. | Pass Vulkan validation, resize/minimize/restore, repeated captures, device/surface failure handling and extended gameplay on supported platforms. |

## Movement, combat and interactions

| Release check | Current status and evidence | Acceptance criteria |
| --- | --- | --- |
| [ ] Basic dry-land movement | **Partial**: 20 Hz standing movement, sprint/jump and swept collision with unit tests. | Differentially compare recorded vanilla movement and server corrections on flat, diagonal, wall/ceiling and ledge courses at matched inputs. |
| [ ] Complete collision and poses | **Missing**: static boxes do not implement all behavior. | Implement step-up, sneaking edges, crouch/crawl/swim dimensions, entity pushing, scaffolding and conditional shapes without clipping or illegal server-visible movement. |
| [ ] Fluids, climbing, flight and vehicles | **Missing**. | Match swimming, fluid currents, ladders/vines, creative flight, gliding, boats/minecarts/mounts, passengers and dismount placement. |
| [ ] Attributes, effects and block physics | **Partial**: generated friction/factors; default movement attributes and velocity events. | Apply all server modifiers/effects, gravity/drag, knockback, ice/slime/honey, speed/jump factors, restitution and context changes with vanilla timing. |
| [ ] Teleports, respawn and correction | **Partial**: acknowledgements and position/dimension resets exist. | Preserve relative position/rotation/velocity semantics; test death/respawn, portals, delayed corrections and repeated teleports under latency. |
| [ ] Player selection and attack packets | **Partial**: standing player ray, three-block reach, static-shape occlusion, swing/attack/sprint packets. | Match vanilla block-outline/entity picking, every pose, reach modifiers and packet ordering; retain the click's timestamp/orientation when rendering input changes. |
| [ ] Sword/axe/shield PvP | **Missing**: fixture attack receipt is not combat simulation. | Complete cooldowns, critical/sprint attacks, shield use/disable, blocking, armor/effects, knockback, damage feedback and item switching; verify server-authoritative outcomes in duels. |
| [ ] Projectiles and item use | **Missing**. | Match bow/crossbow/trident/throwable charging/release, use durations, consumables, offhand behavior and all vanilla interactive items. |
| [ ] Mining, placement and block interaction | **Missing**. | Match break progress, sequence acknowledgements, tool/state checks, placement context/orientation, use priority, cancellation and correction in survival/creative/adventure. |
| [ ] Inventories, containers and transactions | **Missing**. | Every menu/window, slot/component encoding, cursor stack, drag/split/shift-click, hotbar/offhand, creative actions and server transaction/revision reconciliation. |
| [ ] Crafting and recipes | **Missing**. | Recipe book/search, crafting/output consumption, all workstations, recipe synchronization and invalid/stale transaction recovery. |

## Chat, server UI and the product interface

| Release check | Current status and evidence | Acceptance criteria |
| --- | --- | --- |
| [ ] Chat and signing | **Partial**: system text and unsigned outgoing chat/commands; secure-only sends are refused. | Handle player/signed/system/profileless chat, certificates, signatures, last-seen acknowledgements, filters, reporting data and secure-chat settings without disconnects. |
| [ ] Commands and completion | **Partial**: unsigned slash-command submission. | Consume command trees and permissions, offer tab completion, serialize signed arguments when required and display execution results correctly. |
| [ ] Server overlays and menus | **Missing**. | Correct player list, scoreboards, teams, boss bars, titles/action bars, advancements, recipe toasts, maps, books, signs and server-driven screens. |
| [ ] Vanilla HUD and accessibility | **Partial**: configurable diagnostic/PvP metrics. | Complete hotbar, health/food/armor/effects/experience, crosshair, spectator/creative variants, narrator/subtitles, scale and vanilla client options. |
| [ ] Black-hole menu and TOML styling | **Partial**: native menus/shader background and typed config; full layout/theme-driven UI remains. | Every planned screen/layout/theme/profile is editable through the documented TOML/UI contract; settings accurately affect behavior. |
| [ ] Configuration reliability | **Implemented**: schemas, validation, migration, atomic save and last-valid reload. | Exercise GUI edits plus external invalid/partial changes, restarts and migrations; every exposed field is wired or clearly disabled until supported. |
| [ ] MP4 intro/background | **Missing**: static/shader fallback exists. | Supplied video plays as skippable intro and silent loop; failed/missing file falls back; decoding stays off-render and stops/releases during play. |
| [ ] Keybinds, profiles and full PvP suite | **Partial**: metrics, configured controls and HUD subset. | Complete FPS/ping/CPS/keystrokes, armor/effects, zoom, toggle sprint, crosshair/clutter controls and in-client HUD/profile editing through the SDK. |

## Rust mods, launcher and release qualification

| Release check | Current status and evidence | Acceptance criteria |
| --- | --- | --- |
| [ ] Source project and licensing | **Implemented**: modular Rust workspace, MIT license and `MimoAlexer/Void.rs` Git remote. | Verify the public repository contains the intended source, lockfiles, documentation and CI; exclude credentials, signing keys and game assets. |
| [ ] Wasm runtime | **Implemented**: Wasmtime fuel/memory/host-call limits and HUD/metrics samples. | Compile/load real Rust samples; permission denial, loops, memory growth, malformed pointers and partial callback failure are isolated without client corruption. |
| [ ] Native runtime | **Implemented**: versioned C-compatible tables and trusted example loading. | Verify DLL/SO ABI version/size rejection, explicit trust and restart behavior; never claim native fault isolation or safe forced interruption. |
| [ ] Complete public SDK | **Partial**: HUD/metrics contract, manifests/examples; deep APIs remain missing. | Add lifecycle/events, world queries, keybinds, interaction requests, custom channels/render hooks, settings and version/dependency compatibility using stable boundaries. |
| [ ] In-client mod management | **Partial**: scanning/loading/configuration selections and trust controls exist. | Enable/disable and restore selections; handle incompatible dependencies, changed native binaries, errors and restart requirements; test supported platforms. |
| [ ] Launcher setup and accounts | **Partial**: CLI setup/assets/account APIs and installed/sibling launch. | Finish the full intended user flow, account switching/refresh, assets readiness and offline update-service launch; launcher exits during gameplay. |
| [ ] Signed updates and rollback | **Unverified**: verification/staging/rollback/locking code and tests; no qualified published release. | Publish with the real pinned key; test stable/preview channels, corrupt/wrong-platform artifacts, interrupted activation, running-client refusal and previous-version recovery. |
| [ ] Event Horizon correctness | **Partial**: scheduler/revisions/fairness tests and limited application integration. | Admit only deferrable work; preserve simulation/input/HUD; reject obsolete revisions; measure memory, freshness, queue depth and overruns under sustained load. |
| [ ] Event Horizon advantage | **Unverified**; baseline remains default. | Pass the rendered A/B gates below. Synthetic scores, a shader menu or fixture geometry do not establish superiority. |
| [ ] Windows and Linux support | **Unverified** as a complete product. | Test packaged x86-64 builds on Windows and Linux Wayland/X11 with GPU drivers, credentials, inputs, audio, mods, assets and update lifecycle. |
| [ ] Network robustness and soak | **Unverified**. | Extended vanilla/Paper sessions with controlled latency/jitter/loss, chunk bursts, pack reloads and reconnects; no leaks, stale world state, silent packet loss or crashes. |
| [ ] Build, fuzz and release artifacts | **Partial**: workspace checks and protocol regression tests; fuzz targets provided separately. | Pass formatting, strict Clippy, tests and platform CI at the release commit; run sustained sanitizer-backed fuzzing, retain findings and reproduce the shipped executable identity. |

## Required reference runs

Use the official **26.2 vanilla client** as behavioral/visual reference. Record the
exact SHA-256/version/build of the vanilla server and a compatible **Paper 26.2**
build before testing. Run both a clean Paper setup and an intentional plugin
scenario exercising supported vanilla UI/commands; a plugin's custom client mod
requirement is outside vanilla compatibility. Each applicable row above must pass
on vanilla and Paper, not only on the custom fixture.

1. **Connect and retain state:** status → login → configuration → world; online and
   operator-configured offline mode, encryption/compression, reconfiguration,
   packs, server restart, reconnect and transfer. Save server-side disconnect and
   correction logs alongside client logs.
2. **Explore and interact:** all dimensions, chunk boundaries/distances, unloaded
   regions, light changes, every registry's default type and representative state
   variants; block updates, inventories/workstations, mobs/vehicles and item use.
3. **PvP comparisons:** sword/axe/shield duels, sprint/jump attacks, armor/effects,
   projectiles, knockback, death/respawn, swaps and interactions near reach/occlusion
   boundaries. Compare authoritative outcomes and packet-visible timing, not just
   animation appearance. Add modified attributes and delayed corrections.
4. **Presentation and inputs:** screenshot/audio/reference comparisons, packs,
   language/font/accessibility variants, camera turns, resize/minimize and every
   supported control binding. Preserve identical rendering settings across runs.
5. **Faults and endurance:** corrupted packets/files, mod faults, expired sessions,
   temporary service outages and repeated install/update/rollback cycles. Record
   memory/VRAM, handles, queue growth and recovery after an extended play session.

## Performance and default-enablement gates

On the documented i7-14700HX / RTX 5060 Laptop / approximately 32 GB machine, record
power profile and driver. At 1080p, 12 chunks, vanilla textures, fast graphics,
clouds off, default PvP HUD and VSync off, run three 120-second measurements after
warm-up in a fixed duel arena and ordinary-world traversal. Measure loading/pack
transitions separately; also qualify minimum/maximum distance for correctness.

- [ ] At least **240 average FPS**, steady-state p99 **≤8.33 ms**, streaming p99
  **≤16.67 ms**, with p99.9/max stalls, CPU/GPU times, memory and VRAM recorded.
- [ ] Identical scenes/settings for baseline, frame coordination and full Event
  Horizon, in matched 240 FPS pacing tests and separate uncapped capacity tests.
- [ ] Full Event Horizon improves stressed p99 frame time by **≥20%** or p95
  input-to-submit by **≥15%**, regresses neither metric by more than **5%**, retains
  the capacity target and limits uncapped FPS regression to **5%**.
- [ ] Visual correctness holds; nearby-update freshness is not materially worse
  (the comparison tool uses a maximum 5% regression in median p95 update age).
- [ ] A compatible **pinned Sodium build** is compared separately. The internal
  three-mode comparison identifies the scheduler's contribution.
- [ ] Input-to-photon claims have an external measurement setup. Event timestamps
  to CPU submission are labeled input-to-submit.
- [ ] Failed/incomplete evidence leaves Event Horizon experimental and baseline
  as the default. See [the exact evidence schema and gates](EVENT_HORIZON.md).

## Provenance and implementation changelog

**2026-09-26 — initial development implementation, not a complete release.**

- Verified Mojang's [26.2 manifest](https://piston-meta.mojang.com/v1/packages/33c420747ce582e48dff1d8c5d8e67e5bb6257c9/26.2.json),
  official jar hashes and generated packet/state reports. Corrected 26.2 details
  including fluid counts, fixed palette arrays, compact velocities, section
  VarLongs and new sprint ordinals; see [protocol provenance](PROTOCOL.md) and the
  [generated packet inventory](../data/protocol-26.2.toml).
- Added independent wire codecs, bounded networking, socket integration tests and
  official golden data; no vanilla/Paper play certification is inferred.
- Added restricted 20 Hz movement from inspected official coefficients and
  [generated collision/physics data](PHYSICS.md); implemented
  [diagnostic entities and action-time targeting](ENTITIES.md).
- Added [direct Vulkan rendering](RENDERING.md), [experimental scheduling](EVENT_HORIZON.md),
  [Rust mod runtimes](MODDING.md), [account services](ACCOUNTS.md), and
  [launcher/update mechanisms](LAUNCHER.md), with their limitations recorded.
- Added [separate codec/NBT fuzz targets and seed instructions](../fuzz/README.md).
  Supplying targets is not evidence that a coverage-guided campaign ran.

The official jars and assets remain outside the repository. This document is the
full release checklist; [STATUS.md](STATUS.md) is the shorter implementation
overview. Update both from observed evidence as compatibility work lands.
