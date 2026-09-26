# Movement implementation status

`void_core::physics` implements a restricted dry-land standing-player movement
path at 20 ticks/second: keyboard movement, existing sprint state, jump cooldown,
sprint jump impulse, gravity, ground/air drag, and swept AABB collision clipping.
It accepts world-coordinate collision boxes from generated block shapes through
`CollisionWorld`. Unknown terrain postpones prediction without mutating the body.
Server corrections must replace predicted position, velocity, and ground state.

This is **not full vanilla movement**. Missing paths include step-up, sneak edge
protection, crawling/other poses, fluids, ladders, vehicles, gliding, abilities,
effects, context-dependent collision shapes, block restitution/bounce, block
speed/jump factors, entity pushing, and full attribute/modifier synchronization.
Sprinting permission remains the caller's responsibility. Default attributes are
appropriate only for an ordinary player with no modifiers. Do not advertise
anti-cheat compatibility or exact prediction from these unit tests.

The implementation's coefficients and ordering were inspected using JDK25
`javap -c -p` against Mojang's official 26.2 client JAR, SHA1
`2dc72797acbc1b63fc16a11c4ac393605f453754`:

| Reference method | Behavior used |
| --- | --- |
| `Player.createAttributes` | Movement speed `0.10000000149011612` |
| `Attributes` static initializer | Gravity `0.08`; jump strength `0.41999998688697815` |
| `LivingEntity` static initializer | Sprint total multiplier addition `0.30000001192092896` |
| `LivingEntity.jumpFromGround` | `max(old_y, jump_strength)`; sprint horizontal impulse `0.2` |
| `LivingEntity.aiStep` | Ten-tick held-jump cooldown; horizontal squared stop threshold `9e-6`; vertical stop threshold `0.003` |
| `LivingEntity.getFrictionInfluencedSpeed` | Ground acceleration `speed * 0.21600002 / friction^3` for friction above `0.6`; otherwise speed directly |
| `LivingEntity.travelInAir` | Move first; subtract gravity; horizontal drag `friction * 0.91f`; vertical drag `0.98f` |
| `Player.getFlyingSpeed` | Ordinary airborne acceleration `0.02f`; sprint `0.025999999f` |
| `KeyboardInput.tick` and `LocalPlayer.modifyInput` | Normalize keyboard vector, scale `0.98f`, apply unit-square compensation |
| `Direction.axisStepOrder` | Y first; then greater-magnitude horizontal axis |
| `EntityTypes` player registration | Standing width `0.6f`, height `1.8f` |
| `Mth.sin` / `Mth.cos` | 65536-entry float sine table, factor `10430.378350470453` |

Float-to-double conversions are retained where material. The Rust sine table
uses the platform math library to generate the same sampled angles; cross-runtime
bit-identical transcendental results have not been demonstrated. The collision
routine handles flattened AABBs rather than reproducing every voxel-shape detail.

Tests cover jump apex/landing, grounded movement and sprint ratio, stopping,
thin-wall/ceiling sweeps, diagonal normalization/yaw, and atomic handling of
unloaded/invalid collision data. Differential recorded vanilla/server movement
tests remain required before claiming full behavioral compatibility.
