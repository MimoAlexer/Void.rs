# Diagnostic entities and restricted attack targeting

`void-client/src/entities.rs` connects the ECS entity snapshots to the temporary
Vulkan geometry view. These boxes are diagnostic markers, not vanilla models,
skins or animation. Players use the standing 0.6 × 1.8 box; other entity types use
a visibly distinct generic marker. The own-player entity is excluded, and output
is capped at 4,096 entities / 147,456 vertices in stable network-ID order.

`mesh(&ClientWorld, own_id)` returns `Vec<void_render::Vertex>` for this frame.
`attack_target(&ClientWorld, own_id, PlayerPosition, &Terrain)` returns the nearest
other player intersected by the supplied eye ray, or `None`. Kind **156** is the
player ID in the official generated 26.2 registry.

The targeting subset uses standing eye height 1.62, standing player bounds, and
three-block reach measured to the first hit on the box. It tests the exact
yaw/pitch passed with the click, not a later render-camera sample. Terrain blocks
the ray using the official static collision-shape table; missing terrain blocks
target selection. Low slabs do not become full-cube occluders.

The caller must retain that same `PlayerPosition` for the movement/orientation
packet sent immediately before the attack. If the bounded command queue rejects
the movement, do not enqueue the attack with a stale server orientation.

This is **not complete vanilla combat or picking**. Poses, metadata-derived
dimensions, changing reach attributes, non-player interactions, block-outline
picking, entity-context-dependent shapes, cooldowns, damage, shields, effects and
prediction remain separate compatibility work. A successful attack packet merely
submits an interaction to the authoritative server. The local fixture only logs it.

Tests cover reach at and beyond three blocks, action orientation, nearest-player
selection, own-player exclusion, full-block versus slab occlusion, missing terrain,
non-finite input and bounded mesh output. Run:

```text
cargo test -p void-client entities::tests
```
