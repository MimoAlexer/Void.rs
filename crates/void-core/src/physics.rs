//! Restricted 26.2 dry-land player movement, at exactly 20 simulation ticks/sec.
//! Coefficients/order were checked against the official 26.2 client bytecode.
//! This is not complete vanilla physics: see docs/PHYSICS.md for limitations.

use std::sync::OnceLock;
use thiserror::Error;

pub const TICK_SECONDS: f64 = 0.05;
pub const PLAYER_WIDTH: f64 = 0.6_f32 as f64;
pub const PLAYER_HEIGHT: f64 = 1.8_f32 as f64;
const SHAPE_EPSILON: f64 = 1.0e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}
impl Aabb {
    pub fn at_feet(position: [f64; 3]) -> Self {
        let half = PLAYER_WIDTH / 2.0;
        Self {
            min: [position[0] - half, position[1], position[2] - half],
            max: [
                position[0] + half,
                position[1] + PLAYER_HEIGHT,
                position[2] + half,
            ],
        }
    }
    pub fn intersects(self, other: Self) -> bool {
        (0..3).all(|i| self.max[i] > other.min[i] && self.min[i] < other.max[i])
    }
    pub fn translated(self, displacement: [f64; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i] + displacement[i]),
            max: std::array::from_fn(|i| self.max[i] + displacement[i]),
        }
    }
    pub fn swept(self, displacement: [f64; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i] + displacement[i].min(0.0)),
            max: std::array::from_fn(|i| self.max[i] + displacement[i].max(0.0)),
        }
    }
    pub fn is_valid(self) -> bool {
        (0..3).all(|i| {
            self.min[i].is_finite() && self.max[i].is_finite() && self.min[i] <= self.max[i]
        })
    }
}

pub trait CollisionWorld {
    /// Return world-coordinate block collision boxes intersecting the swept
    /// query. Flatten multi-box voxel shapes; do not use visual block bounds.
    fn collision_boxes(&self, query: Aabb) -> Vec<Aabb>;
    /// Friction of the supporting block. Default is ordinary dry solid ground.
    fn friction_at(&self, _feet: [f64; 3]) -> f32 {
        0.6
    }
    /// A false result postpones prediction instead of treating unloaded chunks
    /// as air. Caller can retry after receiving terrain/server correction.
    fn is_loaded(&self, _query: Aabb) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerBody {
    pub position: [f64; 3],
    /// Blocks per simulation tick, not blocks per second.
    pub velocity: [f64; 3],
    pub on_ground: bool,
    pub jump_cooldown: u8,
}
impl PlayerBody {
    pub fn bounds(self) -> Aabb {
        Aabb::at_feet(self.position)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MovementInput {
    /// Keyboard impulse: -1 backward, 0 neutral, +1 forward.
    pub forward: f32,
    /// Keyboard impulse: -1 right, 0 neutral, +1 left.
    pub strafe: f32,
    pub jump: bool,
    /// Already-authorized sprint state; caller enforces food/item/server rules.
    pub sprint: bool,
    pub yaw_degrees: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MovementAttributes {
    pub movement_speed: f32,
    pub jump_strength: f32,
    pub gravity: f64,
    pub friction_modifier: f32,
    pub air_drag_modifier: f32,
}
impl Default for MovementAttributes {
    fn default() -> Self {
        Self {
            movement_speed: 0.1,
            jump_strength: 0.42,
            gravity: 0.08,
            friction_modifier: 1.0,
            air_drag_modifier: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementOutcome {
    pub displacement: [f64; 3],
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    pub jumped: bool,
}

#[derive(Debug, Error, Clone, Copy, Eq, PartialEq)]
pub enum PhysicsError {
    #[error("movement state, input, or attributes are invalid/non-finite")]
    InvalidState,
    #[error("terrain required by movement has not loaded")]
    UnloadedTerrain,
    #[error("collision world returned an invalid shape")]
    InvalidShape,
    #[error("movement exceeds supported swept-query bounds; authoritative correction required")]
    SweepLimit,
    #[error("collision query exceeds the 65536-box safety ceiling")]
    CollisionLimit,
}

pub fn tick(
    body: &mut PlayerBody,
    input: MovementInput,
    world: &impl CollisionWorld,
) -> Result<MovementOutcome, PhysicsError> {
    tick_with_attributes(body, input, world, MovementAttributes::default())
}

/// Performs one fixed tick. Errors leave the body unchanged. Attributes represent
/// the server's effective base movement values before the sprint modifier.
pub fn tick_with_attributes(
    body: &mut PlayerBody,
    input: MovementInput,
    world: &impl CollisionWorld,
    attributes: MovementAttributes,
) -> Result<MovementOutcome, PhysicsError> {
    if body
        .position
        .iter()
        .chain(body.velocity.iter())
        .any(|v| !v.is_finite())
        || !input.forward.is_finite()
        || !input.strafe.is_finite()
        || !input.yaw_degrees.is_finite()
        || !attributes.movement_speed.is_finite()
        || attributes.movement_speed < 0.0
        || !attributes.jump_strength.is_finite()
        || attributes.jump_strength < 0.0
        || !attributes.gravity.is_finite()
        || !attributes.friction_modifier.is_finite()
        || !attributes.air_drag_modifier.is_finite()
    {
        return Err(PhysicsError::InvalidState);
    }
    let mut next = *body;
    next.jump_cooldown = next.jump_cooldown.saturating_sub(1);
    if next.velocity[0] * next.velocity[0] + next.velocity[2] * next.velocity[2] < 9.0e-6 {
        next.velocity[0] = 0.0;
        next.velocity[2] = 0.0;
    }
    if next.velocity[1].abs() < 0.003 {
        next.velocity[1] = 0.0;
    }
    let radians = input.yaw_degrees * 0.017_453_292_f32;
    let (sin, cos) = vanilla_sin_cos(f64::from(radians));
    let jumped = input.jump
        && next.on_ground
        && next.jump_cooldown == 0
        && attributes.jump_strength > 1.0e-5;
    if jumped {
        next.velocity[1] = next.velocity[1].max(f64::from(attributes.jump_strength));
        if input.sprint {
            next.velocity[0] -= f64::from(sin) * 0.2;
            next.velocity[2] += f64::from(cos) * 0.2;
        }
        next.jump_cooldown = 10;
    } else if !input.jump {
        next.jump_cooldown = 0;
    }
    let friction = if next.on_ground {
        let friction = world.friction_at(next.position);
        if !friction.is_finite() || !(0.0..=1.0).contains(&friction) {
            return Err(PhysicsError::InvalidState);
        }
        modified_friction(friction, attributes.friction_modifier)
    } else {
        1.0
    };
    let speed = if next.on_ground {
        let speed = if input.sprint {
            (f64::from(attributes.movement_speed) * (1.0 + f64::from(0.3_f32))) as f32
        } else {
            attributes.movement_speed
        };
        if f64::from(friction) > 0.6 {
            speed * (0.216_000_02_f32 / (friction * friction * friction))
        } else {
            speed
        }
    } else if input.sprint {
        0.025_999_999
    } else {
        0.02
    };
    let [strafe, forward] = keyboard_input(input.strafe, input.forward);
    let mut relative = [f64::from(strafe), f64::from(forward)];
    let squared = relative[0] * relative[0] + relative[1] * relative[1];
    if squared >= 1.0e-7 {
        if squared > 1.0 {
            let length = squared.sqrt();
            relative[0] /= length;
            relative[1] /= length;
        }
        relative[0] *= f64::from(speed);
        relative[1] *= f64::from(speed);
        next.velocity[0] += relative[0] * f64::from(cos) - relative[1] * f64::from(sin);
        next.velocity[2] += relative[1] * f64::from(cos) + relative[0] * f64::from(sin);
    }
    if next
        .velocity
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 64.0)
        || next.position.iter().any(|v| v.abs() > 32_000_000.0)
    {
        return Err(PhysicsError::SweepLimit);
    }
    let requested = next.velocity;
    let bounds = next.bounds();
    let query = bounds.swept(requested);
    if !world.is_loaded(query) {
        return Err(PhysicsError::UnloadedTerrain);
    }
    let shapes = world.collision_boxes(query);
    if shapes.len() > 65536 {
        return Err(PhysicsError::CollisionLimit);
    }
    if shapes.iter().any(|shape| !shape.is_valid()) {
        return Err(PhysicsError::InvalidShape);
    }
    let displacement = collide(bounds, requested, &shapes);
    let horizontal_collision = (requested[0] - displacement[0]).abs() >= 1.0e-5
        || (requested[2] - displacement[2]).abs() >= 1.0e-5;
    let vertical_collision = requested[1] != displacement[1];
    next.on_ground = vertical_collision && requested[1] < 0.0;
    for axis in 0..3 {
        next.position[axis] += displacement[axis];
        if requested[axis] != displacement[axis] {
            next.velocity[axis] = 0.0;
        }
    }
    let horizontal_drag = friction * modified_friction(0.91, attributes.air_drag_modifier);
    let vertical_drag = modified_friction(0.98, attributes.air_drag_modifier);
    next.velocity[0] *= f64::from(horizontal_drag);
    next.velocity[1] = (next.velocity[1] - attributes.gravity) * f64::from(vertical_drag);
    next.velocity[2] *= f64::from(horizontal_drag);
    if next.velocity.iter().any(|v| !v.is_finite()) {
        return Err(PhysicsError::InvalidState);
    }
    *body = next;
    Ok(MovementOutcome {
        displacement,
        horizontal_collision,
        vertical_collision,
        jumped,
    })
}

fn modified_friction(base: f32, modifier: f32) -> f32 {
    (1.0 - (1.0 - base) * modifier).clamp(0.0, 1.0)
}

/// 26.2 KeyboardInput normalizes first. LocalPlayer scales by .98, then maps
/// length relative to a unit square back to the unit disk. Diagonal input thus
/// differs slightly from simply multiplying a normalized direction by .98.
fn keyboard_input(strafe: f32, forward: f32) -> [f32; 2] {
    let x = strafe.clamp(-1.0, 1.0);
    let y = forward.clamp(-1.0, 1.0);
    let length = (x * x + y * y).sqrt();
    if length < 1.0e-4 {
        return [0.0, 0.0];
    }
    let x = x / length * 0.98;
    let y = y / length * 0.98;
    let length = (x * x + y * y).sqrt();
    let dx = x / length;
    let dy = y / length;
    let ratio = if dy.abs() > dx.abs() {
        dx.abs() / dy.abs()
    } else {
        dy.abs() / dx.abs()
    };
    let square_distance = (1.0 + ratio * ratio).sqrt();
    let scale = (length * square_distance).min(1.0);
    [dx * scale, dy * scale]
}

fn vanilla_sin_cos(radians: f64) -> (f32, f32) {
    const SCALE: f64 = 10_430.378_350_470_453;
    static TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..65536)
            .map(|index| (f64::from(index) / SCALE).sin() as f32)
            .collect()
    });
    let sin = ((radians * SCALE) as i64 & 65535) as usize;
    let cos = ((radians * SCALE + 16384.0) as i64 & 65535) as usize;
    (table[sin], table[cos])
}

/// Swept axis clipping prevents tunneling through a thin box. Vertical resolves
/// first, then the larger horizontal component, matching Direction.axisStepOrder.
pub fn collide(bounds: Aabb, requested: [f64; 3], shapes: &[Aabb]) -> [f64; 3] {
    let order = if requested[0].abs() < requested[2].abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    let mut moved = bounds;
    let mut resolved = [0.0; 3];
    for axis in order {
        let mut delta = requested[axis];
        if delta.abs() < SHAPE_EPSILON {
            continue;
        }
        for shape in shapes {
            if (0..3).filter(|i| *i != axis).any(|i| {
                moved.max[i] <= shape.min[i] + SHAPE_EPSILON
                    || moved.min[i] >= shape.max[i] - SHAPE_EPSILON
            }) {
                continue;
            }
            if delta > 0.0 && moved.max[axis] <= shape.min[axis] + SHAPE_EPSILON {
                delta = delta.min((shape.min[axis] - moved.max[axis]).max(0.0));
            } else if delta < 0.0 && moved.min[axis] >= shape.max[axis] - SHAPE_EPSILON {
                delta = delta.max((shape.max[axis] - moved.min[axis]).min(0.0));
            }
        }
        resolved[axis] = delta;
        moved.min[axis] += delta;
        moved.max[axis] += delta;
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scene {
        shapes: Vec<Aabb>,
        loaded: bool,
        friction: f32,
    }
    impl CollisionWorld for Scene {
        fn collision_boxes(&self, _query: Aabb) -> Vec<Aabb> {
            self.shapes.clone()
        }
        fn is_loaded(&self, _query: Aabb) -> bool {
            self.loaded
        }
        fn friction_at(&self, _feet: [f64; 3]) -> f32 {
            self.friction
        }
    }
    fn floor() -> Scene {
        Scene {
            shapes: vec![Aabb {
                min: [-100.0, -1.0, -100.0],
                max: [100.0, 0.0, 100.0],
            }],
            loaded: true,
            friction: 0.6,
        }
    }
    #[test]
    fn grounded_walk_sprint_and_stop_have_bounded_physical_speeds() {
        let scene = floor();
        let mut walk = PlayerBody {
            on_ground: true,
            ..Default::default()
        };
        let mut sprint = walk;
        for _ in 0..100 {
            tick(
                &mut walk,
                MovementInput {
                    forward: 1.0,
                    ..Default::default()
                },
                &scene,
            )
            .unwrap();
            tick(
                &mut sprint,
                MovementInput {
                    forward: 1.0,
                    sprint: true,
                    ..Default::default()
                },
                &scene,
            )
            .unwrap();
        }
        assert_eq!(walk.position[1], 0.0);
        assert!(walk.on_ground);
        let walk_z = walk.position[2];
        let sprint_z = sprint.position[2];
        assert!((sprint_z / walk_z - 1.3).abs() < 0.001);
        for _ in 0..30 {
            tick(&mut walk, MovementInput::default(), &scene).unwrap();
        }
        assert_eq!(walk.velocity[0], 0.0);
        assert_eq!(walk.velocity[2], 0.0);
    }
    #[test]
    fn jump_and_landing_follow_verified_gravity_order() {
        let scene = floor();
        let mut body = PlayerBody {
            on_ground: true,
            ..Default::default()
        };
        let jumped = tick(
            &mut body,
            MovementInput {
                jump: true,
                ..Default::default()
            },
            &scene,
        )
        .unwrap();
        assert!(jumped.jumped);
        assert_eq!(body.position[1], f64::from(0.42_f32));
        assert_eq!(
            body.velocity[1],
            (f64::from(0.42_f32) - 0.08) * f64::from(0.98_f32)
        );
        let mut apex = body.position[1];
        for _ in 0..30 {
            tick(&mut body, MovementInput::default(), &scene).unwrap();
            apex = apex.max(body.position[1]);
        }
        assert!((1.24..1.26).contains(&apex));
        assert_eq!(body.position[1], 0.0);
        assert!(body.on_ground);
    }
    #[test]
    fn high_speed_sweep_hits_thin_wall_and_ceiling() {
        let bounds = Aabb::at_feet([0.0, 0.0, 0.0]);
        let wall = Aabb {
            min: [2.0, -1.0, -5.0],
            max: [2.01, 4.0, 5.0],
        };
        let displacement = collide(bounds, [10.0, 0.0, 0.0], &[wall]);
        assert!((displacement[0] - (2.0 - PLAYER_WIDTH / 2.0)).abs() < 1.0e-12);
        let ceiling = Aabb {
            min: [-2.0, 2.0, -2.0],
            max: [2.0, 3.0, 2.0],
        };
        let displacement = collide(bounds, [0.0, 5.0, 0.0], &[ceiling]);
        assert!((displacement[1] - (2.0 - PLAYER_HEIGHT)).abs() < 1.0e-12);
    }
    #[test]
    fn invalid_and_unloaded_world_do_not_mutate_prediction() {
        let mut body = PlayerBody {
            position: [1.0, 3.0, 2.0],
            on_ground: true,
            ..Default::default()
        };
        let previous = body;
        let mut scene = floor();
        scene.loaded = false;
        assert_eq!(
            tick(&mut body, MovementInput::default(), &scene),
            Err(PhysicsError::UnloadedTerrain)
        );
        assert_eq!(body, previous);
        scene.loaded = true;
        scene.shapes[0].max[1] = f64::NAN;
        assert_eq!(
            tick(&mut body, MovementInput::default(), &scene),
            Err(PhysicsError::InvalidShape)
        );
        assert_eq!(body, previous);
    }
    #[test]
    fn diagonal_input_and_yaw_use_vanilla_orientation() {
        let scene = floor();
        let mut straight = PlayerBody {
            on_ground: true,
            ..Default::default()
        };
        let mut diagonal = straight;
        let mut rotated = straight;
        tick(
            &mut straight,
            MovementInput {
                forward: 1.0,
                ..Default::default()
            },
            &scene,
        )
        .unwrap();
        tick(
            &mut diagonal,
            MovementInput {
                forward: 1.0,
                strafe: 1.0,
                ..Default::default()
            },
            &scene,
        )
        .unwrap();
        tick(
            &mut rotated,
            MovementInput {
                forward: 1.0,
                yaw_degrees: 90.0,
                ..Default::default()
            },
            &scene,
        )
        .unwrap();
        let diagonal_distance = diagonal.position[0].hypot(diagonal.position[2]);
        assert!((1.0..1.03).contains(&(diagonal_distance / straight.position[2])));
        assert!((rotated.position[0] + straight.position[2]).abs() < 1.0e-5);
        assert!(rotated.position[2].abs() < 1.0e-5);
    }
}
