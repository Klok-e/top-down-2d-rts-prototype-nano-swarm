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
    obstacles: Vec<Obstacle>,
    revision: u64,
    chunks: Mutex<HashMap<IVec2, Chunk>>,
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
            obstacles,
            revision: 1,
            chunks: Mutex::new(HashMap::new()),
        }
    }
    /// Replace physical geometry and invalidate cached connectivity when it changes.
    /// Paint changes affect future costs without discarding physical connectivity.
    pub fn refresh(&mut self, grid: &IntentGrid, obstacles: Vec<Obstacle>) -> bool {
        let mut replacement = Self::new(grid, obstacles);
        if replacement.min == self.min
            && replacement.max == self.max
            && replacement.obstacles == self.obstacles
        {
            return false;
        }
        replacement.revision = self.revision.wrapping_add(1);
        *self = replacement;
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
            && self.obstacles.iter().all(|o| match *o {
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
            })
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
    fn regions(&self, chunk: IVec2) -> Chunk {
        let mut cache = self.chunks.lock().expect("navigation chunk cache poisoned");
        cache
            .entry(chunk)
            .or_insert_with(|| {
                let mut regions = HashMap::new();
                for y in 0..CHUNK {
                    for x in 0..CHUNK {
                        let cell = chunk * CHUNK + IVec2::new(x, y);
                        if regions.contains_key(&cell) || !self.point_clear(Self::center(cell)) {
                            continue;
                        }
                        let mut queue = VecDeque::from([cell]);
                        regions.insert(cell, cell);
                        while let Some(current) = queue.pop_front() {
                            for d in DIRECTIONS {
                                let next = current + d;
                                if Self::chunk(next) == chunk
                                    && !regions.contains_key(&next)
                                    && self.segment_clear(Self::center(current), Self::center(next))
                                {
                                    regions.insert(next, cell);
                                    queue.push_back(next);
                                }
                            }
                        }
                    }
                }
                Chunk { regions }
            })
            .clone()
    }
    fn region(&self, cell: IVec2) -> Option<IVec2> {
        let key = Self::chunk(cell);
        if let Some(chunk) = self
            .chunks
            .lock()
            .expect("navigation chunk cache poisoned")
            .get(&key)
        {
            return chunk.regions.get(&cell).copied();
        }
        self.regions(key).regions.get(&cell).copied()
    }
    fn connections(&self, node: IVec2) -> Vec<(IVec2, IVec2, IVec2)> {
        let chunk = self.regions(Self::chunk(node));
        let mut result = Vec::new();
        for (&cell, &region) in &chunk.regions {
            if region != node {
                continue;
            }
            for d in DIRECTIONS {
                let next = cell + d;
                if Self::chunk(next) == Self::chunk(cell)
                    || !self.segment_clear(Self::center(cell), Self::center(next))
                {
                    continue;
                }
                if let Some(other) = self.region(next) {
                    result.push((other, cell, next));
                }
            }
        }
        result
    }
    fn connectors(&self, p: Vec2) -> Vec<IVec2> {
        let cell = Self::cell(p);
        let mut result = Vec::new();
        for y in -1..=1 {
            for x in -1..=1 {
                let c = cell + IVec2::new(x, y);
                if self.segment_clear(p, Self::center(c)) {
                    result.push(c);
                }
            }
        }
        result
    }
    fn multiplier(grid: &IntentGrid, swarm: SwarmId, hauler: bool, cell: IVec2) -> f32 {
        if hauler
            && grid
                .cell(world_to_cell(Self::center(cell)))
                .is_some_and(|c| c.visible_to(IntentKind::Corridor, swarm))
        {
            0.35
        } else {
            1.0
        }
    }
    fn segment_cost(
        start: Vec2,
        end: Vec2,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> f32 {
        if !hauler {
            return start.distance(end);
        }
        let delta = end - start;
        let mut splits = vec![0.0, 1.0];
        for axis in 0..2 {
            if delta[axis].abs() < f32::EPSILON {
                continue;
            }
            let lo = (start[axis].min(end[axis]) / CELL_WIDTH).floor() as i32 + 1;
            let hi = (start[axis].max(end[axis]) / CELL_WIDTH).ceil() as i32;
            for edge in lo..hi {
                splits.push((edge as f32 * CELL_WIDTH - start[axis]) / delta[axis]);
            }
        }
        splits.sort_by(f32::total_cmp);
        splits
            .windows(2)
            .map(|pair| {
                let midpoint = start + delta * ((pair[0] + pair[1]) * 0.5);
                (pair[1] - pair[0])
                    * delta.length()
                    * Self::multiplier(grid, swarm, hauler, Self::cell(midpoint))
            })
            .sum()
    }
    /// Reach a clear position inside a movement range without requiring access to the target center.
    pub fn route_within_range(
        &self,
        start: Vec2,
        center: Vec2,
        radius: f32,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        if !radius.is_finite() || radius < 0.0 || !center.is_finite() {
            return RouteOutcome::Unreachable;
        }
        if start.distance(center) <= radius && self.point_clear(start) {
            return RouteOutcome::Found(Route {
                waypoints: vec![start],
                cost: 0.0,
            });
        }
        let nearest = center + (start - center).normalize_or_zero() * (radius - 0.5).max(0.0);
        if let RouteOutcome::Found(route) = self.route(start, nearest, grid, swarm, hauler) {
            return RouteOutcome::Found(route);
        }
        let min = Self::cell(center - Vec2::splat(radius));
        let max = Self::cell(center + Vec2::splat(radius));
        let mut candidates = Vec::new();
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let p = Self::center(IVec2::new(x, y));
                if p.distance(center) <= radius && self.point_clear(p) {
                    candidates.push(p);
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        for p in candidates {
            if let RouteOutcome::Found(route) = self.route(start, p, grid, swarm, hauler) {
                return RouteOutcome::Found(route);
            }
        }
        RouteOutcome::Unreachable
    }

    /// Reach any accessible exterior work position, including alternative faces.
    pub fn route_to_interaction(
        &self,
        start: Vec2,
        region: crate::nanobot::InteractionRegion,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        if self.point_clear(start) && region.contains(start) {
            return RouteOutcome::Found(Route {
                waypoints: vec![start],
                cost: 0.0,
            });
        }
        for endpoint in region.candidates(start) {
            if let RouteOutcome::Found(route) = self.route(start, endpoint, grid, swarm, hauler) {
                return RouteOutcome::Found(route);
            }
        }
        RouteOutcome::Unreachable
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
    pub fn route(
        &self,
        start: Vec2,
        end: Vec2,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        if !self.point_clear(start) || !self.point_clear(end) {
            return RouteOutcome::Unreachable;
        }
        if (start.distance_squared(end) < 0.0001 || !hauler) && self.segment_clear(start, end) {
            return RouteOutcome::Found(Route {
                waypoints: vec![end],
                cost: start.distance(end),
            });
        }
        let direct = self.segment_clear(start, end).then(|| Route {
            waypoints: vec![end],
            cost: Self::segment_cost(start, end, grid, swarm, hauler),
        });
        let starts = self.connectors(start);
        let ends = self.connectors(end);
        if starts.is_empty() || ends.is_empty() {
            return direct.map_or(RouteOutcome::Unreachable, RouteOutcome::Found);
        }
        let goals: HashSet<_> = ends.iter().filter_map(|&c| self.region(c)).collect();
        let mut open = BinaryHeap::new();
        let mut costs = HashMap::new();
        let mut previous = HashMap::new();
        let scale = if hauler { 0.35 } else { 1.0 };
        for &c in &starts {
            if let Some(region) = self.region(c) {
                costs.insert(region, 0.0);
                open.push(Visit {
                    score: Self::center(region).distance(end) * scale,
                    cost: 0.0,
                    node: region,
                });
            }
        }
        // Probe the destination component alongside the forward search. A small
        // sealed work area must not require exploring the entire exterior world.
        let sources: HashSet<_> = starts.iter().filter_map(|&c| self.region(c)).collect();
        let mut reverse_seen = goals.clone();
        let mut reverse_queue: VecDeque<_> = goals.iter().copied().collect();
        let mut reverse_connected = false;
        let mut reached = None;
        while let Some(Visit { node, cost, .. }) = open.pop() {
            if cost > costs[&node] {
                continue;
            }
            if goals.contains(&node) {
                reached = Some(node);
                break;
            }
            if !reverse_connected {
                let Some(reverse) = reverse_queue.pop_front() else {
                    return direct.map_or(RouteOutcome::Unreachable, RouteOutcome::Found);
                };
                if sources.contains(&reverse) {
                    reverse_connected = true;
                } else {
                    for (other, _, _) in self.connections(reverse) {
                        if reverse_seen.insert(other) {
                            reverse_queue.push_back(other);
                        }
                    }
                }
            }
            let mut neighbors: HashMap<IVec2, f32> = HashMap::new();
            for (other, cell, next) in self.connections(node) {
                let distance = Self::center(node).distance(Self::center(other));
                let weight = (Self::multiplier(grid, swarm, hauler, cell)
                    + Self::multiplier(grid, swarm, hauler, next))
                    * 0.5;
                neighbors
                    .entry(other)
                    .and_modify(|v| *v = v.min(distance * weight))
                    .or_insert(distance * weight);
            }
            for (next, edge) in neighbors {
                let candidate = cost + edge;
                if candidate < *costs.get(&next).unwrap_or(&f32::INFINITY) {
                    costs.insert(next, candidate);
                    previous.insert(next, node);
                    open.push(Visit {
                        score: candidate + Self::center(next).distance(end) * scale,
                        cost: candidate,
                        node: next,
                    });
                }
            }
        }
        let Some(mut node) = reached else {
            return direct.map_or(RouteOutcome::Unreachable, RouteOutcome::Found);
        };
        let mut allowed = HashSet::from([node]);
        while let Some(&parent) = previous.get(&node) {
            allowed.insert(parent);
            node = parent;
        }
        // Every coarse edge represents an actual fine-grid edge. Restricting detail to
        // the connected regions on that chain therefore preserves reachability.
        open.clear();
        costs.clear();
        previous.clear();
        for &c in &starts {
            if self.region(c).is_some_and(|r| allowed.contains(&r)) {
                let cost = Self::segment_cost(start, Self::center(c), grid, swarm, hauler);
                costs.insert(c, cost);
                open.push(Visit {
                    score: cost + Self::center(c).distance(end) * scale,
                    cost,
                    node: c,
                });
            }
        }
        let mut best = f32::INFINITY;
        let mut finish = None;
        while let Some(Visit { node, cost, score }) = open.pop() {
            if score >= best {
                break;
            }
            if cost > costs[&node] {
                continue;
            }
            if ends.contains(&node) {
                let total = cost + Self::segment_cost(Self::center(node), end, grid, swarm, hauler);
                if total < best {
                    best = total;
                    finish = Some(node);
                }
            }
            for d in DIRECTIONS {
                let next = node + d;
                if !self.region(next).is_some_and(|r| allowed.contains(&r))
                    || !self.segment_clear(Self::center(node), Self::center(next))
                {
                    continue;
                }
                let weight = (Self::multiplier(grid, swarm, hauler, node)
                    + Self::multiplier(grid, swarm, hauler, next))
                    * 0.5;
                let candidate = cost + CELL_WIDTH * d.as_vec2().length() * weight;
                if candidate < *costs.get(&next).unwrap_or(&f32::INFINITY) {
                    costs.insert(next, candidate);
                    previous.insert(next, node);
                    open.push(Visit {
                        score: candidate + Self::center(next).distance(end) * scale,
                        cost: candidate,
                        node: next,
                    });
                }
            }
        }
        if let Some(route) = direct
            && route.cost <= best
        {
            return RouteOutcome::Found(route);
        }
        let Some(mut node) = finish else {
            return RouteOutcome::Unreachable;
        };
        let mut waypoints = vec![end, Self::center(node)];
        while let Some(&parent) = previous.get(&node) {
            node = parent;
            waypoints.push(Self::center(node));
        }
        waypoints.reverse();
        RouteOutcome::Found(Route {
            waypoints,
            cost: best,
        })
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
