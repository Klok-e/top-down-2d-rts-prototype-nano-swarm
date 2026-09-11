use super::{BODY_RADIUS, CELL_WIDTH, Obstacle};
use crate::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{SwarmId, world_to_cell},
};
use bevy::platform::collections::HashMap as FastHashMap;
use bevy::prelude::*;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    sync::Mutex,
};

const CHUNK: i32 = 8;
const DIRECTIONS: [IVec2; 8] = [
    IVec2::new(-1, 0),
    IVec2::new(1, 0),
    IVec2::new(0, -1),
    IVec2::new(0, 1),
    IVec2::new(-1, -1),
    IVec2::new(-1, 1),
    IVec2::new(1, -1),
    IVec2::new(1, 1),
];
#[derive(Debug, Clone)]
pub struct Route {
    pub waypoints: Vec<Vec2>,
    pub cost: f32,
}
#[derive(Debug, Clone)]
pub enum RouteOutcome {
    Found(Route),
    Unreachable,
}
type ComponentEdges = FastHashMap<IVec2, Vec<(IVec2, IVec2, IVec2)>>;

/// Shared physical connectivity and clearance for all swarms and Nanobot Types.
/// Chunk regions are materialized only where searches visit the large world.
#[derive(Resource)]
pub struct Navigation {
    min: Vec2,
    max: Vec2,
    obstacles: std::sync::Arc<Vec<Obstacle>>,
    obstacle_index: std::sync::Arc<ObstacleIndex>,
    clearing: Vec<Obstacle>,
    revision: u64,
    chunks: std::sync::Arc<Mutex<FastHashMap<IVec2, Chunk>>>,
    connections: std::sync::Arc<Mutex<ComponentEdges>>,
    scheduler: Mutex<budget::Scheduler>,
    chunk_builds: std::sync::Arc<Mutex<FastHashMap<IVec2, budget::ChunkBuild>>>,
    expansions: std::sync::Arc<budget::ExpansionCounters>,
}
/// Bucket coverage is capped for both shapes and queries. Huge shapes remain
/// global candidates; long segments fall back to the bounded obstacle list.
#[derive(Default)]
struct ObstacleIndex {
    buckets: FastHashMap<IVec2, Vec<usize>>,
    global: Vec<usize>,
    count: usize,
}
impl ObstacleIndex {
    const MAX_BUCKETS: i64 = 64;
    fn bounds(a: Vec2, b: Vec2) -> (IVec2, IVec2) {
        let width = CELL_WIDTH * CHUNK as f32;
        (
            (a.min(b) / width).floor().as_ivec2(),
            (a.max(b) / width).floor().as_ivec2(),
        )
    }
    fn small_span(min: IVec2, max: IVec2) -> bool {
        let width = i64::from(max.x) - i64::from(min.x) + 1;
        let height = i64::from(max.y) - i64::from(min.y) + 1;
        width <= Self::MAX_BUCKETS
            && height <= Self::MAX_BUCKETS
            && width * height <= Self::MAX_BUCKETS
    }
    fn new(obstacles: &[Obstacle]) -> Self {
        let mut index = Self {
            count: obstacles.len(),
            ..Default::default()
        };
        for (id, shape) in obstacles.iter().enumerate() {
            let (center, half) = match *shape {
                Obstacle::Rectangle { center, half } => (center, half),
                Obstacle::Circle { center, radius } => (center, Vec2::splat(radius)),
            };
            let half = half + Vec2::splat(BODY_RADIUS);
            let (min, max) = Self::bounds(center - half, center + half);
            if !Self::small_span(min, max) {
                index.global.push(id);
                continue;
            }
            for y in min.y..=max.y {
                for x in min.x..=max.x {
                    index.buckets.entry(IVec2::new(x, y)).or_default().push(id);
                }
            }
        }
        index
    }
    fn candidates(&self, a: Vec2, b: Vec2) -> Vec<usize> {
        let (min, max) = Self::bounds(a, b);
        if !Self::small_span(min, max) {
            return (0..self.count).collect();
        }
        let mut candidates = self.global.clone();
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                if let Some(ids) = self.buckets.get(&IVec2::new(x, y)) {
                    candidates.extend(ids);
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}
#[derive(Clone)]
struct Chunk {
    regions: FastHashMap<IVec2, IVec2>,
}
#[derive(Clone, Copy, PartialEq)]
struct Visit {
    score: f32,
    cost: f32,
    node: IVec2,
}
impl Eq for Visit {}
impl Ord for Visit {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| other.cost.total_cmp(&self.cost))
            .then_with(|| self.node.y.cmp(&other.node.y))
            .then_with(|| self.node.x.cmp(&other.node.x))
    }
}
impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Default for Navigation {
    fn default() -> Self {
        Self::new(&IntentGrid::new(0, 0), vec![])
    }
}
impl Navigation {
    pub fn new(grid: &IntentGrid, obstacles: Vec<Obstacle>) -> Self {
        let min = IVec2::new(-(grid.width() / 2), -(grid.height() / 2)).as_vec2() * ZONE_BLOCK_SIZE;
        Self {
            min,
            max: min + Vec2::new(grid.width() as f32, grid.height() as f32) * ZONE_BLOCK_SIZE,
            obstacle_index: std::sync::Arc::new(ObstacleIndex::new(&obstacles)),
            obstacles: std::sync::Arc::new(obstacles),
            clearing: Vec::new(),
            revision: 1,
            chunks: std::sync::Arc::new(Mutex::new(FastHashMap::default())),
            connections: Default::default(),
            scheduler: Mutex::new(budget::Scheduler::default()),
            expansions: Default::default(),
            chunk_builds: Default::default(),
        }
    }
    /// Replace physical geometry and invalidate cached connectivity when it changes.
    /// Paint changes affect future costs without discarding physical connectivity.
    pub fn refresh(&mut self, grid: &IntentGrid, obstacles: Vec<Obstacle>) -> bool {
        let min = IVec2::new(-(grid.width() / 2), -(grid.height() / 2)).as_vec2() * ZONE_BLOCK_SIZE;
        let max = min + Vec2::new(grid.width() as f32, grid.height() as f32) * ZONE_BLOCK_SIZE;
        if min == self.min && max == self.max && obstacles == *self.obstacles {
            return false;
        }
        let replacement = Self::new(grid, obstacles);
        let changed: Vec<_> = self
            .obstacles
            .iter()
            .filter(|o| !replacement.obstacles.contains(o))
            .chain(
                replacement
                    .obstacles
                    .iter()
                    .filter(|o| !self.obstacles.contains(o)),
            )
            .copied()
            .collect();
        if self.min != replacement.min || self.max != replacement.max {
            self.chunks.lock().unwrap().clear();
        } else {
            self.chunks.lock().unwrap().retain(|chunk, _| {
                let lower =
                    (*chunk * CHUNK).as_vec2() * CELL_WIDTH - Vec2::splat(CELL_WIDTH + BODY_RADIUS);
                let upper = ((*chunk + IVec2::ONE) * CHUNK).as_vec2() * CELL_WIDTH
                    + Vec2::splat(CELL_WIDTH + BODY_RADIUS);
                !changed.iter().any(|shape| {
                    let (center, half) = match *shape {
                        Obstacle::Rectangle { center, half } => (center, half),
                        Obstacle::Circle { center, radius } => (center, Vec2::splat(radius)),
                    };
                    (center + half).cmpge(lower).all() && (center - half).cmple(upper).all()
                })
            });
        }
        self.chunk_builds.lock().unwrap().clear();
        self.connections.lock().unwrap().clear();
        self.min = replacement.min;
        self.max = replacement.max;
        self.obstacles = replacement.obstacles;
        self.obstacle_index = replacement.obstacle_index;
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn point_clear(&self, p: Vec2) -> bool {
        p.is_finite()
            && p.cmpge(self.min).all()
            && p.cmplt(self.max).all()
            && self
                .obstacle_index
                .candidates(p, p)
                .into_iter()
                .all(|id| self.obstacles[id].admits_body(p))
    }
    /// Analytic swept-disc clearance covers every point of the movement segment.
    pub fn segment_clear(&self, a: Vec2, b: Vec2) -> bool {
        self.point_clear(a)
            && self.point_clear(b)
            && self
                .obstacle_index
                .candidates(a, b)
                .into_iter()
                .all(|id| self.obstacles[id].segment_clear(a, b))
    }
    /// Swept travel respects clearing entry barriers while allowing existing occupants to leave.
    pub(crate) fn movement_clear(&self, start: Vec2, end: Vec2) -> bool {
        self.segment_clear(start, end)
            && self
                .clearing
                .iter()
                .all(|shape| !shape.admits_body(start) || shape.segment_clear(start, end))
    }

    fn center(cell: IVec2) -> Vec2 {
        (cell.as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH
    }
    fn cell(p: Vec2) -> IVec2 {
        (p / CELL_WIDTH).floor().as_ivec2()
    }
    fn chunk(cell: IVec2) -> IVec2 {
        IVec2::new(cell.x.div_euclid(CHUNK), cell.y.div_euclid(CHUNK))
    }
    /// Immediate query for isolated hypothetical geometry, outside the runtime request queue.
    pub fn route(
        &self,
        start: Vec2,
        end: Vec2,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        self.solve(start, RouteGoal::Point(end), grid, swarm, hauler)
    }
    pub fn route_to_interaction(
        &self,
        start: Vec2,
        region: crate::nanobot::InteractionRegion,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        self.solve(start, RouteGoal::Interaction(region), grid, swarm, hauler)
    }
    pub fn route_within_range(
        &self,
        start: Vec2,
        center: Vec2,
        radius: f32,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        self.solve(
            start,
            RouteGoal::Range { center, radius },
            grid,
            swarm,
            hauler,
        )
    }
    pub fn route_cost(
        &self,
        start: Vec2,
        end: Vec2,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> Option<f32> {
        match self.route(start, end, grid, swarm, hauler) {
            RouteOutcome::Found(route) => Some(route.cost),
            RouteOutcome::Unreachable => None,
        }
    }
}
fn point_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let delta = b - a;
    let length = delta.length_squared();
    if length == 0.0 {
        return p.distance(a);
    }
    p.distance(a + delta * ((p - a).dot(delta) / length).clamp(0.0, 1.0))
}
fn segment_box(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> bool {
    let delta = b - a;
    let mut lo: f32 = 0.0;
    let mut hi: f32 = 1.0;
    for axis in 0..2 {
        if delta[axis].abs() < f32::EPSILON {
            if a[axis] <= min[axis] || a[axis] >= max[axis] {
                return false;
            }
        } else {
            let first = (min[axis] - a[axis]) / delta[axis];
            let last = (max[axis] - a[axis]) / delta[axis];
            lo = lo.max(first.min(last));
            hi = hi.min(first.max(last));
            if lo >= hi {
                return false;
            }
        }
    }
    lo < hi
}

#[path = "budget.rs"]
mod budget;
pub use budget::*;

impl Obstacle {
    pub fn segment_clear(self, a: Vec2, b: Vec2) -> bool {
        match self {
            Obstacle::Circle { center, radius } => {
                point_segment_distance(center, a, b) >= radius + BODY_RADIUS
            }
            Obstacle::Rectangle { center, half } => {
                let min = center - half;
                let max = center + half;
                // A rectangle expanded by a disc consists of two strips and four round corners.
                !segment_box(
                    a,
                    b,
                    min - Vec2::new(BODY_RADIUS, 0.0),
                    max + Vec2::new(BODY_RADIUS, 0.0),
                ) && !segment_box(
                    a,
                    b,
                    min - Vec2::new(0.0, BODY_RADIUS),
                    max + Vec2::new(0.0, BODY_RADIUS),
                ) && [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)]
                    .iter()
                    .all(|&corner| point_segment_distance(corner, a, b) >= BODY_RADIUS)
            }
        }
    }
}
