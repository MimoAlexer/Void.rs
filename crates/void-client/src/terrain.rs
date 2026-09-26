//! Temporary block-state geometry view. This does not claim vanilla model/lighting parity.
use std::{collections::HashMap, sync::Arc};
use void_protocol::ChunkData;
use void_render::Vertex;

pub struct Terrain {
    pub chunks: HashMap<(i32, i32), Arc<ChunkData>>,
    pub revision: u64,
}
impl Terrain {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            revision: 0,
        }
    }
    pub fn insert(&mut self, chunk: ChunkData) {
        self.chunks.insert((chunk.x, chunk.z), Arc::new(chunk));
        self.revision += 1;
    }
    pub fn remove(&mut self, x: i32, z: i32) {
        self.chunks.remove(&(x, z));
        self.revision += 1;
    }
}

/// Mesh only surfaces with an air neighbour. Runs on a worker from an immutable snapshot.
pub fn mesh(
    chunks: &HashMap<(i32, i32), Arc<ChunkData>>,
    center: [f64; 3],
    radius: u8,
) -> Vec<Vertex> {
    let mut vertices = Vec::new();
    let directions = [
        (
            [1, 0, 0],
            [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
            0.76,
        ),
        (
            [-1, 0, 0],
            [[0., 0., 1.], [0., 1., 1.], [0., 1., 0.], [0., 0., 0.]],
            0.66,
        ),
        (
            [0, 1, 0],
            [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]],
            1.0,
        ),
        (
            [0, -1, 0],
            [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]],
            0.48,
        ),
        (
            [0, 0, 1],
            [[1., 0., 1.], [1., 1., 1.], [0., 1., 1.], [0., 0., 1.]],
            0.82,
        ),
        (
            [0, 0, -1],
            [[0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]],
            0.72,
        ),
    ];
    let cx = (center[0].floor() as i32).div_euclid(16);
    let cz = (center[2].floor() as i32).div_euclid(16);
    let mut ordered: Vec<_> = chunks.values().collect();
    ordered.sort_by_key(|c| {
        let dx = i64::from(c.x) - i64::from(cx);
        let dz = i64::from(c.z) - i64::from(cz);
        (dx * dx + dz * dz, c.x, c.z)
    });
    for chunk in ordered {
        let dx = chunk.x - (center[0].floor() as i32).div_euclid(16);
        let dz = chunk.z - (center[2].floor() as i32).div_euclid(16);
        if dx.abs() > i32::from(radius) || dz.abs() > i32::from(radius) {
            continue;
        }
        for section in &chunk.sections {
            if section.non_air_count == 0 {
                continue;
            }
            for y in 0..16 {
                for z in 0..16 {
                    for x in 0..16 {
                        let id = section.block_states[(y * 16 + z) * 16 + x];
                        if is_air(id) {
                            continue;
                        }
                        let p = [
                            chunk.x * 16 + x as i32,
                            section.y * 16 + y as i32,
                            chunk.z * 16 + z as i32,
                        ];
                        let base = state_color(id);
                        for (d, corners, shade) in &directions {
                            if !is_air(block(chunks, [p[0] + d[0], p[1] + d[1], p[2] + d[2]])) {
                                continue;
                            }
                            for i in [0, 1, 2, 0, 2, 3] {
                                let c = corners[i];
                                vertices.push(Vertex {
                                    position: [
                                        p[0] as f32 + c[0],
                                        p[1] as f32 + c[1],
                                        p[2] as f32 + c[2],
                                    ],
                                    color: base.map(|v| v * shade),
                                });
                            }
                            if vertices.len() >= 3_900_000 {
                                return vertices;
                            }
                        }
                    }
                }
            }
        }
    }
    vertices
}
pub fn block(chunks: &HashMap<(i32, i32), Arc<ChunkData>>, p: [i32; 3]) -> u32 {
    let Some(chunk) = chunks.get(&(p[0].div_euclid(16), p[2].div_euclid(16))) else {
        return 0;
    };
    let Some(section) = chunk.sections.iter().find(|s| s.y == p[1].div_euclid(16)) else {
        return 0;
    };
    section.block_states
        [((p[1].rem_euclid(16) * 16 + p[2].rem_euclid(16)) * 16 + p[0].rem_euclid(16)) as usize]
}
fn state_color(id: u32) -> [f32; 3] {
    // Deliberately diagnostic color, not guessed vanilla textures.
    let h = id.wrapping_mul(2654435761);
    [
        0.22 + (h & 255) as f32 / 900.,
        0.26 + ((h >> 8) & 255) as f32 / 900.,
        0.24 + ((h >> 16) & 255) as f32 / 1000.,
    ]
}
fn is_air(id: u32) -> bool {
    void_protocol::block_states::info(id).is_some_and(|b| b.air)
}

impl void_core::physics::CollisionWorld for Terrain {
    fn friction_at(&self, feet: [f64; 3]) -> f32 {
        void_protocol::block_states::friction(block(
            &self.chunks,
            [
                feet[0].floor() as i32,
                (feet[1] - 0.5000001).floor() as i32,
                feet[2].floor() as i32,
            ],
        ))
    }
    fn collision_boxes(&self, query: void_core::physics::Aabb) -> Vec<void_core::physics::Aabb> {
        let min = query.min.map(|v| v.floor() as i32 - 1);
        let max = query.max.map(|v| v.ceil() as i32 + 1);
        let volume = (0..3)
            .map(|i| i64::from(max[i]) - i64::from(min[i]) + 1)
            .try_fold(1_i64, |a, b| a.checked_mul(b));
        if volume.is_none_or(|v| v > 65536) {
            return vec![void_core::physics::Aabb {
                min: [f64::NAN; 3],
                max: [f64::NAN; 3],
            }];
        }
        let mut boxes = vec![];
        for x in min[0]..=max[0] {
            for y in min[1]..=max[1] {
                for z in min[2]..=max[2] {
                    for shape in
                        void_protocol::block_states::collision_boxes(block(&self.chunks, [x, y, z]))
                    {
                        let bounds = void_core::physics::Aabb {
                            min: [
                                x as f64 + shape[0] as f64,
                                y as f64 + shape[1] as f64,
                                z as f64 + shape[2] as f64,
                            ],
                            max: [
                                x as f64 + shape[3] as f64,
                                y as f64 + shape[4] as f64,
                                z as f64 + shape[5] as f64,
                            ],
                        };
                        if query.intersects(bounds) {
                            boxes.push(bounds);
                        }
                    }
                }
            }
        }
        boxes
    }
    fn is_loaded(&self, query: void_core::physics::Aabb) -> bool {
        let min = query.min.map(|v| (v.floor() as i32).div_euclid(16));
        let max = query.max.map(|v| (v.floor() as i32).div_euclid(16));
        for x in min[0]..=max[0] {
            for z in min[2]..=max[2] {
                if !self.chunks.contains_key(&(x, z)) {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use void_protocol::ChunkSection;
    #[test]
    fn adjacent_blocks_hide_shared_faces() {
        let mut states = vec![0; 4096];
        states[0] = 1;
        states[1] = 1;
        let data = ChunkData {
            x: 0,
            z: 0,
            min_y: 0,
            sections: vec![ChunkSection {
                y: 0,
                non_air_count: 2,
                fluid_count: 0,
                block_states: states,
                biomes: vec![0; 64],
            }],
        };
        let chunks = HashMap::from([((0, 0), Arc::new(data))]);
        assert_eq!(mesh(&chunks, [0.; 3], 2).len(), 60);
    }
}
