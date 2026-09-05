use super::{BODY_RADIUS, CELL_WIDTH, Obstacle};
use crate::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{SwarmId, world_to_cell},
};
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
/// Shared physical connectivity and clearance for all swarms and Nanobot Types.
/// Chunk regions are materialized only where searches visit the large world.
#[derive(Resource)]
pub struct Navigation {
    min: Vec2,
    max: Vec2,
    obstacles: std::sync::Arc<Vec<Obstacle>>,
    clearing: Vec<Obstacle>,
    revision: u64,
    chunks: std::sync::Arc<Mutex<HashMap<IVec2, Chunk>>>,
    scheduler: Mutex<budget::Scheduler>,
}
#[derive(Clone)]
struct Chunk {
    regions: HashMap<IVec2, IVec2>,
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
            obstacles: std::sync::Arc::new(obstacles),
            clearing: Vec::new(),
            revision: 1,
            chunks: std::sync::Arc::new(Mutex::new(HashMap::new())),
            scheduler: Mutex::new(budget::Scheduler::default()),
        }
    }
    /// Replace physical geometry and invalidate cached connectivity when it changes.
    /// Paint changes affect future costs without discarding physical connectivity.
    pub fn refresh(&mut self, grid: &IntentGrid, obstacles: Vec<Obstacle>) -> bool {
        let replacement = Self::new(grid, obstacles);
        if replacement.min == self.min
            && replacement.max == self.max
            && replacement.obstacles == self.obstacles
        {
            return false;
        }
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
        self.min = replacement.min;
        self.max = replacement.max;
        self.obstacles = replacement.obstacles;
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
            && self.obstacles.iter().all(|o| o.admits_body(p))
    }
    /// Analytic swept-disc clearance covers every point of the movement segment.
    pub fn segment_clear(&self, a: Vec2, b: Vec2) -> bool {
        self.point_clear(a)
            && self.point_clear(b)
            && self.obstacles.iter().all(|o| o.segment_clear(a, b))
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
        self.admits_body(a)
            && self.admits_body(b)
            && match self {
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
