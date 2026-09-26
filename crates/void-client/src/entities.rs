//! Diagnostic entity boxes and restricted standing-player targeting.
//!
//! This is not vanilla entity rendering or complete combat: skins, poses, entity
//! metadata, reach attributes, vehicles and combat prediction remain separate work.
//! Target selection only uses the yaw/pitch supplied with the input action. The
//! caller must send that same orientation before the resulting attack packet.
use crate::terrain::Terrain;
use void_core::{
    physics::{Aabb, CollisionWorld},
    world::{ClientWorld, EntitySnapshot},
};
use void_protocol::PlayerPosition;
use void_render::Vertex;

const PLAYER_KIND: i32 = 156;
const MAX_DIAGNOSTIC_ENTITIES: usize = 4096;
const REACH: f64 = 3.0;
const EYE_HEIGHT: f64 = 1.62_f32 as f64;
const EPSILON: f64 = 1.0e-7;

fn valid_coordinates(position: [f64; 3]) -> bool {
    position
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= 32_000_000.0)
}

fn bounds(entity: &EntitySnapshot) -> Aabb {
    let feet = [entity.position.x, entity.position.y, entity.position.z];
    if entity.kind == PLAYER_KIND {
        Aabb::at_feet(feet)
    } else {
        // A distinct generic marker, explicitly not a guessed entity hitbox/model.
        Aabb {
            min: [feet[0] - 0.25, feet[1], feet[2] - 0.25],
            max: [feet[0] + 0.25, feet[1] + 0.5, feet[2] + 0.25],
        }
    }
}

pub fn mesh(world: &ClientWorld, own: i32) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(
        world
            .entity_count()
            .min(MAX_DIAGNOSTIC_ENTITIES)
            .saturating_mul(36),
    );
    for entity in world
        .snapshot()
        .into_iter()
        .filter(|entity| entity.id != own)
        .take(MAX_DIAGNOSTIC_ENTITIES)
    {
        if !valid_coordinates([entity.position.x, entity.position.y, entity.position.z]) {
            continue;
        }
        let color = if entity.kind == PLAYER_KIND {
            [0.68, 0.31, 0.92]
        } else {
            [0.30, 0.72, 0.76]
        };
        append_box(&mut vertices, bounds(&entity), color);
    }
    vertices
}

fn append_box(vertices: &mut Vec<Vertex>, bounds: Aabb, color: [f32; 3]) {
    let faces = [
        ([[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]], 0.76),
        ([[0, 0, 1], [0, 1, 1], [0, 1, 0], [0, 0, 0]], 0.66),
        ([[0, 1, 1], [1, 1, 1], [1, 1, 0], [0, 1, 0]], 1.00),
        ([[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], 0.48),
        ([[1, 0, 1], [1, 1, 1], [0, 1, 1], [0, 0, 1]], 0.82),
        ([[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]], 0.72),
    ];
    for (corners, shade) in faces {
        for index in [0, 1, 2, 0, 2, 3] {
            vertices.push(Vertex {
                position: std::array::from_fn(|axis| {
                    if corners[index][axis] == 0 {
                        bounds.min[axis] as f32
                    } else {
                        bounds.max[axis] as f32
                    }
                }),
                color: color.map(|channel| channel * shade),
            });
        }
    }
}

/// Returns the nearest standing player intersected by this action's eye ray,
/// within three blocks and before any loaded terrain collision shape.
/// Does not change position, consume input, choose a new camera sample, or send packets.
pub fn attack_target(
    world: &ClientWorld,
    own: i32,
    position: PlayerPosition,
    terrain: &Terrain,
) -> Option<i32> {
    if !valid_coordinates([position.x, position.y, position.z])
        || !position.yaw.is_finite()
        || !position.pitch.is_finite()
    {
        return None;
    }
    let origin = [position.x, position.y + EYE_HEIGHT, position.z];
    let yaw = f64::from(position.yaw).to_radians();
    let pitch = f64::from(position.pitch).to_radians();
    let direction = [
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    ];
    let (distance, target) = world
        .snapshot()
        .into_iter()
        .filter(|entity| entity.id != own)
        .take(MAX_DIAGNOSTIC_ENTITIES)
        .filter(|entity| entity.kind == PLAYER_KIND)
        .filter_map(|entity| {
            ray_box(origin, direction, bounds(&entity), REACH).map(|distance| (distance, entity.id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?;

    let end: [f64; 3] = std::array::from_fn(|axis| origin[axis] + direction[axis] * distance);
    let query = Aabb {
        min: std::array::from_fn(|axis| origin[axis].min(end[axis]) - EPSILON),
        max: std::array::from_fn(|axis| origin[axis].max(end[axis]) + EPSILON),
    };
    // Missing terrain cannot be treated as a transparent path to a target.
    if !terrain.is_loaded(query) {
        return None;
    }
    for collision in terrain.collision_boxes(query) {
        if !collision.is_valid()
            || ray_box(origin, direction, collision, distance + EPSILON).is_some()
        {
            return None;
        }
    }
    Some(target)
}

fn ray_box(origin: [f64; 3], direction: [f64; 3], bounds: Aabb, limit: f64) -> Option<f64> {
    if !bounds.is_valid() {
        return None;
    }
    let mut enter = 0.0_f64;
    let mut exit = limit;
    for axis in 0..3 {
        if direction[axis].abs() < 1.0e-12 {
            if origin[axis] < bounds.min[axis] || origin[axis] > bounds.max[axis] {
                return None;
            }
            continue;
        }
        let a = (bounds.min[axis] - origin[axis]) / direction[axis];
        let b = (bounds.max[axis] - origin[axis]) / direction[axis];
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    Some(enter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use void_core::world::{Orientation, Position, ServerEvent, Velocity};
    use void_protocol::{ChunkData, ChunkSection};

    fn terrain(wall: Option<u32>) -> Terrain {
        let mut terrain = Terrain::new();
        let mut states = vec![0; 4096];
        if let Some(state) = wall {
            states[(2 * 16 + 9) * 16 + 8] = state;
        }
        terrain.insert(ChunkData {
            x: 0,
            z: 0,
            min_y: 0,
            sections: vec![ChunkSection {
                y: 0,
                non_air_count: u16::from(wall.is_some()),
                fluid_count: 0,
                block_states: states,
                biomes: vec![0; 64],
            }],
        });
        terrain
    }
    fn spawn(world: &mut ClientWorld, id: i32, kind: i32, x: f64, z: f64) {
        world
            .apply(
                world.next_sequence(),
                ServerEvent::Spawn(EntitySnapshot {
                    id,
                    kind,
                    position: Position { x, y: 1.0, z },
                    velocity: Velocity::default(),
                    orientation: Orientation::default(),
                }),
            )
            .unwrap();
    }
    fn position() -> PlayerPosition {
        PlayerPosition {
            x: 8.0,
            y: 1.0,
            z: 8.0,
            ..Default::default()
        }
    }

    #[test]
    fn reach_is_measured_to_player_box_and_uses_action_orientation() {
        let mut world = ClientWorld::new();
        spawn(&mut world, 2, PLAYER_KIND, 8.0, 11.3);
        let terrain = terrain(None);
        assert_eq!(attack_target(&world, 1, position(), &terrain), Some(2));
        world
            .apply(world.next_sequence(), ServerEvent::Despawn { id: 2 })
            .unwrap();
        spawn(&mut world, 2, PLAYER_KIND, 8.0, 11.301);
        assert_eq!(attack_target(&world, 1, position(), &terrain), None);
        spawn(&mut world, 3, PLAYER_KIND, 5.5, 8.0);
        let at_click = PlayerPosition {
            yaw: 90.0,
            ..position()
        };
        assert_eq!(attack_target(&world, 1, at_click, &terrain), Some(3));
        assert_eq!(at_click.yaw, 90.0);
        assert_eq!(attack_target(&world, 1, position(), &terrain), None);
    }

    #[test]
    fn chooses_nearest_other_player_and_ignores_generic_markers() {
        let mut world = ClientWorld::new();
        spawn(&mut world, 1, PLAYER_KIND, 8.0, 8.0);
        spawn(&mut world, 2, PLAYER_KIND, 8.0, 10.5);
        spawn(&mut world, 3, PLAYER_KIND, 8.0, 9.5);
        spawn(&mut world, 4, 151, 8.0, 8.5);
        assert_eq!(
            attack_target(&world, 1, position(), &terrain(None)),
            Some(3)
        );
        assert_eq!(mesh(&world, 1).len(), 3 * 36);
    }

    #[test]
    fn official_full_block_occludes_but_lower_slab_does_not() {
        let mut world = ClientWorld::new();
        spawn(&mut world, 2, PLAYER_KIND, 8.0, 10.5);
        assert_eq!(
            attack_target(&world, 1, position(), &terrain(Some(1))),
            None
        );
        let bottom_slab = (0..void_protocol::block_states::STATE_COUNT as u32)
            .find(|state| {
                void_protocol::block_states::info(*state)
                    .is_some_and(|info| info.name == "minecraft:stone_slab")
                    && void_protocol::block_states::collision_boxes(*state)
                        == [[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
            })
            .unwrap();
        assert_eq!(
            attack_target(&world, 1, position(), &terrain(Some(bottom_slab))),
            Some(2)
        );
        assert_eq!(attack_target(&world, 1, position(), &Terrain::new()), None);
    }

    #[test]
    fn rejects_nonfinite_action_and_bounds_mesh_output() {
        let mut world = ClientWorld::new();
        for id in 0..MAX_DIAGNOSTIC_ENTITIES as i32 + 20 {
            spawn(&mut world, id, PLAYER_KIND, 8.0, 10.0);
        }
        assert_eq!(mesh(&world, -1).len(), MAX_DIAGNOSTIC_ENTITIES * 36);
        assert_eq!(
            attack_target(
                &world,
                -1,
                PlayerPosition {
                    yaw: f32::NAN,
                    ..position()
                },
                &terrain(None)
            ),
            None
        );
    }
}
