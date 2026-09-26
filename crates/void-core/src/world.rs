//! Server-authoritative entity storage and timestamped input. This deliberately
//! does not pretend to implement vanilla movement physics or combat prediction.

use bevy_ecs::prelude::*;
use std::collections::{HashMap, VecDeque};
use thiserror::Error;

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkEntityId(pub i32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Position {
    fn valid(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Velocity {
    fn valid(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Orientation {
    pub yaw: f32,
    pub pitch: f32,
}
impl Orientation {
    fn valid(self) -> bool {
        self.yaw.is_finite() && self.pitch.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySnapshot {
    pub id: i32,
    pub kind: i32,
    pub position: Position,
    pub velocity: Velocity,
    pub orientation: Orientation,
}

#[derive(Component, Clone, Copy)]
struct EntityKind(i32);

#[derive(Clone, Debug)]
pub enum ServerEvent {
    Spawn(EntitySnapshot),
    Move {
        id: i32,
        position: Position,
        orientation: Orientation,
    },
    Velocity {
        id: i32,
        velocity: Velocity,
    },
    Despawn {
        id: i32,
    },
    /// Clears entity state on dimension switch; this changes the epoch used by
    /// background work even if network IDs are reused by the new dimension.
    Reset,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WorldError {
    #[error("out-of-order update: expected sequence {expected}, got {received}")]
    Sequence { expected: u64, received: u64 },
    #[error("unknown network entity {0}")]
    UnknownEntity(i32),
    #[error("network entity {0} was spawned twice")]
    DuplicateEntity(i32),
    #[error("server supplied non-finite coordinates or rotation")]
    NonFinite,
    #[error("world revision counter exhausted")]
    CounterExhausted,
}

pub struct ClientWorld {
    world: World,
    entities: HashMap<i32, Entity>,
    next_sequence: u64,
    revision: u64,
    epoch: u64,
}

impl Default for ClientWorld {
    fn default() -> Self {
        Self {
            world: World::new(),
            entities: HashMap::new(),
            next_sequence: 0,
            revision: 0,
            epoch: 0,
        }
    }
}

impl ClientWorld {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
    pub fn ecs(&self) -> &World {
        &self.world
    }

    /// Apply ordered network events exactly once. Validation happens before any
    /// mutation, so failed updates do not consume a sequence or leave partial state.
    pub fn apply(&mut self, sequence: u64, event: ServerEvent) -> Result<(), WorldError> {
        if sequence != self.next_sequence {
            return Err(WorldError::Sequence {
                expected: self.next_sequence,
                received: sequence,
            });
        }
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(WorldError::CounterExhausted)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::CounterExhausted)?;
        match event {
            ServerEvent::Spawn(entity) => {
                if !entity.position.valid()
                    || !entity.velocity.valid()
                    || !entity.orientation.valid()
                {
                    return Err(WorldError::NonFinite);
                }
                if self.entities.contains_key(&entity.id) {
                    return Err(WorldError::DuplicateEntity(entity.id));
                }
                let id = self
                    .world
                    .spawn((
                        NetworkEntityId(entity.id),
                        EntityKind(entity.kind),
                        entity.position,
                        entity.velocity,
                        entity.orientation,
                    ))
                    .id();
                self.entities.insert(entity.id, id);
            }
            ServerEvent::Move {
                id,
                position,
                orientation,
            } => {
                if !position.valid() || !orientation.valid() {
                    return Err(WorldError::NonFinite);
                }
                let entity = *self
                    .entities
                    .get(&id)
                    .ok_or(WorldError::UnknownEntity(id))?;
                self.world
                    .entity_mut(entity)
                    .insert((position, orientation));
            }
            ServerEvent::Velocity { id, velocity } => {
                if !velocity.valid() {
                    return Err(WorldError::NonFinite);
                }
                let entity = *self
                    .entities
                    .get(&id)
                    .ok_or(WorldError::UnknownEntity(id))?;
                self.world.entity_mut(entity).insert(velocity);
            }
            ServerEvent::Despawn { id } => {
                let entity = self
                    .entities
                    .remove(&id)
                    .ok_or(WorldError::UnknownEntity(id))?;
                self.world.despawn(entity);
            }
            ServerEvent::Reset => {
                let epoch = self
                    .epoch
                    .checked_add(1)
                    .ok_or(WorldError::CounterExhausted)?;
                self.world.clear_entities();
                self.entities.clear();
                self.epoch = epoch;
            }
        }
        self.next_sequence = next_sequence;
        self.revision = revision;
        Ok(())
    }

    pub fn entity(&self, id: i32) -> Option<EntitySnapshot> {
        let entity = self.world.entity(*self.entities.get(&id)?);
        Some(EntitySnapshot {
            id,
            kind: entity.get::<EntityKind>()?.0,
            position: *entity.get::<Position>()?,
            velocity: *entity.get::<Velocity>()?,
            orientation: *entity.get::<Orientation>()?,
        })
    }

    /// Snapshot ordering is stable regardless of hash-table iteration order.
    pub fn snapshot(&self) -> Vec<EntitySnapshot> {
        let mut entities: Vec<_> = self
            .entities
            .keys()
            .filter_map(|id| self.entity(*id))
            .collect();
        entities.sort_by_key(|entity| entity.id);
        entities
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    Attack,
    Use,
    ReleaseUse,
    Jump,
    StartSprint,
    StopSprint,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputAction {
    pub sequence: u64,
    pub timestamp_us: u64,
    /// Camera orientation at the instant of the action, not a later render sample.
    pub orientation: Orientation,
    pub kind: ActionKind,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum InputError {
    #[error("input action queue is full; action was not accepted")]
    Full,
    #[error("input action has non-finite orientation")]
    NonFinite,
    #[error("input sequence must increase strictly and timestamps must not decrease")]
    OutOfOrder,
}

pub struct InputQueue {
    capacity: usize,
    pending: VecDeque<InputAction>,
    latest: Option<(u64, u64)>,
}
impl InputQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            pending: VecDeque::with_capacity(capacity.min(4096)),
            latest: None,
        }
    }
    pub fn push(&mut self, action: InputAction) -> Result<(), InputError> {
        if !action.orientation.valid() {
            return Err(InputError::NonFinite);
        }
        if self
            .latest
            .is_some_and(|(seq, time)| action.sequence <= seq || action.timestamp_us < time)
        {
            return Err(InputError::OutOfOrder);
        }
        if self.pending.len() >= self.capacity {
            return Err(InputError::Full);
        }
        self.latest = Some((action.sequence, action.timestamp_us));
        self.pending.push_back(action);
        Ok(())
    }
    pub fn pop(&mut self) -> Option<InputAction> {
        self.pending.pop_front()
    }
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entity(id: i32) -> EntitySnapshot {
        EntitySnapshot {
            id,
            kind: 1,
            position: Position::default(),
            velocity: Velocity::default(),
            orientation: Orientation::default(),
        }
    }
    #[test]
    fn ordered_ecs_updates_are_atomic_and_snapshots_stable() {
        let mut world = ClientWorld::new();
        assert!(world.apply(1, ServerEvent::Spawn(entity(8))).is_err());
        world.apply(0, ServerEvent::Spawn(entity(8))).unwrap();
        assert_eq!(
            world.apply(1, ServerEvent::Spawn(entity(8))),
            Err(WorldError::DuplicateEntity(8))
        );
        assert_eq!(world.next_sequence(), 1);
        world.apply(1, ServerEvent::Spawn(entity(2))).unwrap();
        assert_eq!(
            world.snapshot().iter().map(|e| e.id).collect::<Vec<_>>(),
            [2, 8]
        );
        assert_eq!(
            world.apply(
                2,
                ServerEvent::Move {
                    id: 8,
                    position: Position {
                        x: f64::NAN,
                        ..Default::default()
                    },
                    orientation: Orientation::default()
                }
            ),
            Err(WorldError::NonFinite)
        );
        assert_eq!(world.entity(8).unwrap(), entity(8));
        world.apply(2, ServerEvent::Reset).unwrap();
        assert_eq!(world.entity_count(), 0);
        assert_eq!(world.epoch(), 1);
    }
    #[test]
    fn bounded_input_queue_keeps_event_orientation() {
        let mut queue = InputQueue::new(1);
        let action = InputAction {
            sequence: 4,
            timestamp_us: 100,
            orientation: Orientation {
                yaw: 45.0,
                pitch: 10.0,
            },
            kind: ActionKind::Attack,
        };
        queue.push(action).unwrap();
        assert_eq!(
            queue.push(InputAction {
                sequence: 5,
                ..action
            }),
            Err(InputError::Full)
        );
        assert_eq!(queue.pop(), Some(action));
        assert_eq!(queue.push(action), Err(InputError::OutOfOrder));
        queue
            .push(InputAction {
                sequence: 5,
                ..action
            })
            .unwrap();
    }
}
