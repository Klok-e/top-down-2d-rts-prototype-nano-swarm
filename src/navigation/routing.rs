use super::{BODY_RADIUS, CELL_WIDTH, Obstacle};
use crate::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{SwarmId, world_to_cell},
};
use bevy::platform::collections::{
    HashMap, HashMap as FastHashMap, HashSet, HashSet as FastHashSet,
};
use bevy::prelude::*;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
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
impl Route {
    /// Whether a body walking from `start` along every waypoint stays movement-clear.
    pub(crate) fn still_clear(&self, navigation: &Navigation, start: Vec2) -> bool {
        let mut from = start;
        self.waypoints.iter().all(|&waypoint| {
            let clear = navigation.movement_clear(from, waypoint);
            from = waypoint;
            clear
        })
    }
}
#[derive(Debug, Clone)]
pub enum RouteOutcome {
    Found(Route),
    Unreachable,
}
/// A region's edges: (neighbouring region, own cell, neighbouring cell).
type Edges = std::sync::Arc<[(IVec2, IVec2, IVec2)]>;
type ComponentEdges = FastHashMap<IVec2, Edges>;

/// Shared physical connectivity and clearance for all swarms and Nanobot Types.
/// Chunk regions are materialized only where searches visit the large world.
#[derive(Resource)]
pub struct Navigation {
    min: Vec2,
    max: Vec2,
    obstacles: std::sync::Arc<Vec<Obstacle>>,
    obstacle_index: std::sync::Arc<ObstacleIndex>,
    clearing: Vec<Obstacle>,
    /// Main geometry with every clearing footprint solid, shared by requests starting outside them.
    clearing_layer: Option<std::sync::Arc<Navigation>>,
    /// Main-hierarchy caches this geometry variant reuses where its changed shapes cannot reach.
    base: Option<std::sync::Arc<BaseHierarchy>>,
    revision: u64,
    /// Distinct for every bounds, obstacle set and clearing a hierarchy has held.
    clearance: u64,
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
    /// Per obstacle, the box beyond which it cannot constrain a body, with rounding slack.
    reach: Vec<(Vec2, Vec2)>,
}
impl ObstacleIndex {
    const MAX_BUCKETS: i64 = 64;
    fn bounds(a: Vec2, b: Vec2) -> (IVec2, IVec2) {
        let width = CELL_WIDTH * CHUNK as f32;
        (
            crate::spatial::floor_to_ivec2(a.min(b) / width),
            crate::spatial::floor_to_ivec2(a.max(b) / width),
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
            index.reach.push(shape.reach_box());
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
    /// Whether every obstacle that could constrain the query box satisfies `accepts`.
    /// Same answers as testing every candidate, without collecting them.
    fn all_near(
        &self,
        obstacles: &[Obstacle],
        a: Vec2,
        b: Vec2,
        accepts: impl Fn(Obstacle) -> bool,
    ) -> bool {
        let (low, high) = (a.min(b), a.max(b));
        let near =
            |&id: &usize| super::separated(low, high, self.reach[id]) || accepts(obstacles[id]);
        let (min, max) = Self::bounds(a, b);
        if !Self::small_span(min, max) {
            return (0..self.count).all(|id| near(&id));
        }
        self.global.iter().all(near)
            && (min.y..=max.y).all(|y| {
                (min.x..=max.x).all(|x| {
                    self.buckets
                        .get(&IVec2::new(x, y))
                        .is_none_or(|ids| ids.iter().all(near))
                })
            })
    }
}
/// Chunks and edges of a geometry variant that equal the main hierarchy's.
struct BaseHierarchy {
    navigation: Navigation,
    /// `None` when changed shapes are too large to enumerate their chunks.
    changed_chunks: Option<FastHashSet<IVec2>>,
}
impl BaseHierarchy {
    /// Chunks whose regions changed shapes may alter, enumerated once per variant.
    fn new(navigation: Navigation, changed: &[Obstacle]) -> Self {
        const MAX_CHANGED_CHUNKS: i64 = 4096;
        let width = CELL_WIDTH * CHUNK as f32;
        let mut chunks = FastHashSet::default();
        for &shape in changed {
            let (center, half) = shape.bounds();
            let margin = Vec2::splat(Navigation::CHUNK_MARGIN);
            let min = ((center - half - margin) / width).floor().as_ivec2() - IVec2::ONE;
            let max = ((center + half + margin) / width).floor().as_ivec2();
            let span = (i64::from(max.x) - i64::from(min.x) + 1)
                * (i64::from(max.y) - i64::from(min.y) + 1);
            if !(0..=MAX_CHANGED_CHUNKS).contains(&span) {
                return Self {
                    navigation,
                    changed_chunks: None,
                };
            }
            for y in min.y..=max.y {
                for x in min.x..=max.x {
                    let chunk = IVec2::new(x, y);
                    if Navigation::chunk_touched(chunk, shape) {
                        chunks.insert(chunk);
                    }
                }
            }
        }
        Self {
            navigation,
            changed_chunks: Some(chunks),
        }
    }
    fn shares_regions(&self, chunk: IVec2) -> bool {
        self.changed_chunks
            .as_ref()
            .is_some_and(|changed| !changed.contains(&chunk))
    }
    /// Edges also name regions of neighboring chunks, so those must match too.
    fn shares_edges(&self, chunk: IVec2) -> bool {
        (-1..=1).all(|y| (-1..=1).all(|x| self.shares_regions(chunk + IVec2::new(x, y))))
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
/// A process-wide unique clearance identity.
fn next_clearance() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
            clearing_layer: None,
            base: None,
            revision: 1,
            clearance: next_clearance(),
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
        let changed = changed_shapes(&self.obstacles, &replacement.obstacles);
        if self.min != replacement.min || self.max != replacement.max {
            self.chunks.lock().unwrap().clear();
        } else {
            self.chunks.lock().unwrap().retain(|&chunk, _| {
                !changed
                    .iter()
                    .any(|&shape| Self::chunk_touched(chunk, shape))
            });
        }
        self.chunk_builds.lock().unwrap().clear();
        self.connections.lock().unwrap().clear();
        self.min = replacement.min;
        self.max = replacement.max;
        self.obstacles = replacement.obstacles;
        self.obstacle_index = replacement.obstacle_index;
        self.revision = self.revision.wrapping_add(1);
        self.clearance = next_clearance();
        self.rebuild_clearing_layer();
        true
    }
    /// Chunk regions and their edges cannot change unless a shape reaches this far.
    const CHUNK_MARGIN: f32 = CELL_WIDTH + BODY_RADIUS;
    fn chunk_touched(chunk: IVec2, shape: Obstacle) -> bool {
        let lower = (chunk * CHUNK).as_vec2() * CELL_WIDTH - Vec2::splat(Self::CHUNK_MARGIN);
        let upper =
            ((chunk + IVec2::ONE) * CHUNK).as_vec2() * CELL_WIDTH + Vec2::splat(Self::CHUNK_MARGIN);
        let (center, half) = shape.bounds();
        (center + half).cmpge(lower).all() && (center - half).cmple(upper).all()
    }
    /// Share this hierarchy's geometry and caches under an empty scheduler.
    fn shared_snapshot(&self) -> Navigation {
        Navigation {
            min: self.min,
            max: self.max,
            obstacles: self.obstacles.clone(),
            obstacle_index: self.obstacle_index.clone(),
            clearing: Vec::new(),
            clearing_layer: None,
            base: self.base.clone(),
            revision: self.revision,
            clearance: next_clearance(),
            chunks: self.chunks.clone(),
            connections: self.connections.clone(),
            scheduler: Mutex::new(budget::Scheduler::default()),
            chunk_builds: self.chunk_builds.clone(),
            expansions: self.expansions.clone(),
        }
    }
    /// Geometry replacing this hierarchy's obstacles. Chunks and edges out of reach of
    /// the changed shapes come from this hierarchy; only the rest get private caches.
    fn variant(&self, obstacles: Vec<Obstacle>) -> Navigation {
        debug_assert!(
            self.base.is_none(),
            "variants derive from the main hierarchy"
        );
        let changed = changed_shapes(&self.obstacles, &obstacles);
        Navigation {
            min: self.min,
            max: self.max,
            obstacle_index: std::sync::Arc::new(ObstacleIndex::new(&obstacles)),
            obstacles: std::sync::Arc::new(obstacles),
            clearing: Vec::new(),
            clearing_layer: None,
            base: Some(std::sync::Arc::new(BaseHierarchy::new(
                self.shared_snapshot(),
                &changed,
            ))),
            revision: self.revision,
            clearance: next_clearance(),
            chunks: Default::default(),
            connections: Default::default(),
            scheduler: Mutex::new(budget::Scheduler::default()),
            chunk_builds: Default::default(),
            expansions: self.expansions.clone(),
        }
    }
    fn rebuild_clearing_layer(&mut self) {
        self.clearing_layer = (!self.clearing.is_empty()).then(|| {
            let obstacles = self
                .obstacles
                .iter()
                .chain(&self.clearing)
                .copied()
                .collect();
            std::sync::Arc::new(self.variant(obstacles))
        });
    }
    /// The hierarchy that owns this chunk's regions for this geometry.
    fn region_owner(&self, chunk: IVec2) -> &Navigation {
        match &self.base {
            Some(base) if base.shares_regions(chunk) => &base.navigation,
            _ => self,
        }
    }
    /// The hierarchy that owns the edges leaving this chunk for this geometry.
    fn edge_owner(&self, chunk: IVec2) -> &Navigation {
        match &self.base {
            Some(base) if base.shares_edges(chunk) => &base.navigation,
            _ => self,
        }
    }
    /// An already-built region, without building or waiting.
    fn cached_region(&self, cell: IVec2) -> Option<IVec2> {
        let chunk = Self::chunk(cell);
        self.region_owner(chunk)
            .chunks
            .lock()
            .unwrap()
            .get(&chunk)
            .and_then(|built| built.regions.get(&cell))
            .copied()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Equal for two hierarchies, or one hierarchy at two times, only when bounds,
    /// obstacles and clearing are the same, so every clearance query answers the same.
    pub(crate) fn clearance_identity(&self) -> u64 {
        self.clearance
    }
    pub fn point_clear(&self, p: Vec2) -> bool {
        p.is_finite()
            && p.cmpge(self.min).all()
            && p.cmplt(self.max).all()
            && self
                .obstacle_index
                .all_near(&self.obstacles, p, p, |shape| shape.admits_body(p))
    }
    pub(crate) fn point_clear_for_entry(&self, point: Vec2) -> bool {
        self.point_clear(point) && self.clearing.iter().all(|shape| shape.admits_body(point))
    }
    /// Analytic swept-disc clearance covers every point of the movement segment.
    pub fn segment_clear(&self, a: Vec2, b: Vec2) -> bool {
        self.point_clear(a)
            && self.point_clear(b)
            && self
                .obstacle_index
                .all_near(&self.obstacles, a, b, |shape| shape.segment_clear(a, b))
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
        crate::spatial::floor_to_ivec2(p / CELL_WIDTH)
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
/// Shapes present in only one of two obstacle lists.
fn changed_shapes(old: &[Obstacle], new: &[Obstacle]) -> Vec<Obstacle> {
    let old_bits: FastHashSet<_> = old.iter().map(|shape| shape.bits()).collect();
    let new_bits: FastHashSet<_> = new.iter().map(|shape| shape.bits()).collect();
    old.iter()
        .filter(|shape| !new_bits.contains(&shape.bits()))
        .chain(new.iter().filter(|shape| !old_bits.contains(&shape.bits())))
        .copied()
        .collect()
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

#[path = "tile_visibility.rs"]
mod tile_visibility;

impl Obstacle {
    pub fn segment_clear(self, a: Vec2, b: Vec2) -> bool {
        match self {
            Obstacle::Circle { center, radius } => {
                point_segment_distance(center, a, b) >= radius + BODY_RADIUS
            }
            Obstacle::Rectangle { center, half } => {
                let (strips, corners) = expanded_rectangle(center, half);
                strips
                    .into_iter()
                    .all(|(min, max)| !segment_box(a, b, min, max))
                    && corners
                        .into_iter()
                        .all(|corner| point_segment_distance(corner, a, b) >= BODY_RADIUS)
            }
        }
    }
}

// A rectangle expanded by a disc consists of two strips and four round corners.
fn expanded_rectangle(center: Vec2, half: Vec2) -> ([(Vec2, Vec2); 2], [Vec2; 4]) {
    let min = center - half;
    let max = center + half;
    let horizontal = Vec2::new(BODY_RADIUS, 0.0);
    let vertical = Vec2::new(0.0, BODY_RADIUS);
    (
        [
            (min - horizontal, max + horizontal),
            (min - vertical, max + vertical),
        ],
        [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)],
    )
}
