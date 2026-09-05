use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

#[derive(Default)]
pub(super) struct ExpansionCounters {
    pub hierarchy_cells: AtomicUsize,
    pub hierarchy_chunks: AtomicUsize,
    pub coarse: AtomicUsize,
    pub fine: AtomicUsize,
}
impl ExpansionCounters {
    fn snapshot(&self) -> NavigationWork {
        NavigationWork {
            hierarchy_cells: self.hierarchy_cells.load(AtomicOrdering::Relaxed),
            hierarchy_chunks: self.hierarchy_chunks.load(AtomicOrdering::Relaxed),
            coarse_expansions: self.coarse.load(AtomicOrdering::Relaxed),
            fine_expansions: self.fine.load(AtomicOrdering::Relaxed),
            ..Default::default()
        }
    }
}
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

pub(super) type ChunkBuild = Pin<Box<dyn Future<Output = Chunk> + Send>>;

struct WorkUnit(bool);
impl WorkUnit {
    fn new() -> Self {
        Self(false)
    }
}
impl Future for WorkUnit {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

impl Navigation {
    async fn async_regions(&self, chunk: IVec2) -> Chunk {
        std::future::poll_fn(|context| {
            if let Some(cached) = self.chunks.lock().unwrap().get(&chunk).cloned() {
                return Poll::Ready(cached);
            }
            let mut builds = self.chunk_builds.lock().unwrap();
            let build = builds.entry(chunk).or_insert_with(|| {
                // The build snapshot owns independent empty caches, so retaining a
                // partial build cannot form an Arc cycle back to this shared cache.
                let snapshot = self.hypothetical(self.obstacles.as_ref().clone());
                Box::pin(async move { snapshot.async_build_regions(chunk).await })
            });
            match build.as_mut().poll(context) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => {
                    self.chunks.lock().unwrap().insert(chunk, result.clone());
                    builds.remove(&chunk);
                    Poll::Ready(result)
                }
            }
        })
        .await
    }

    async fn async_build_regions(&self, chunk: IVec2) -> Chunk {
        self.expansions
            .hierarchy_chunks
            .fetch_add(1, AtomicOrdering::Relaxed);
        let mut regions = HashMap::new();
        for y in 0..CHUNK {
            WorkUnit::new().await;
            for x in 0..CHUNK {
                WorkUnit::new().await;
                let cell = chunk * CHUNK + IVec2::new(x, y);
                if regions.contains_key(&cell) || !self.async_point_clear(Self::center(cell)).await
                {
                    continue;
                }
                let mut queue = VecDeque::from([cell]);
                regions.insert(cell, cell);
                while let Some(current) = queue.pop_front() {
                    self.expansions
                        .hierarchy_cells
                        .fetch_add(1, AtomicOrdering::Relaxed);
                    WorkUnit::new().await;
                    for d in DIRECTIONS {
                        WorkUnit::new().await;
                        let next = current + d;
                        if Self::chunk(next) == chunk
                            && !regions.contains_key(&next)
                            && self
                                .async_segment_clear(Self::center(current), Self::center(next))
                                .await
                        {
                            regions.insert(next, cell);
                            queue.push_back(next);
                        }
                    }
                }
            }
        }
        Chunk { regions }
    }

    async fn async_region(&self, cell: IVec2) -> Option<IVec2> {
        let key = Self::chunk(cell);
        if let Some(chunk) = self
            .chunks
            .lock()
            .expect("navigation chunk cache poisoned")
            .get(&key)
        {
            return chunk.regions.get(&cell).copied();
        }
        self.async_regions(key).await.regions.get(&cell).copied()
    }
    async fn async_connections(&self, node: IVec2) -> Vec<(IVec2, IVec2, IVec2)> {
        let chunk = self.async_regions(Self::chunk(node)).await;
        let mut result = Vec::new();
        let mut cells: Vec<_> = chunk
            .regions
            .iter()
            .map(|(&cell, &region)| (cell, region))
            .collect();
        cells.sort_by_key(|(cell, _)| (cell.y, cell.x));
        for (cell, region) in cells {
            WorkUnit::new().await;
            if region != node {
                continue;
            }
            for d in DIRECTIONS {
                WorkUnit::new().await;
                let next = cell + d;
                if Self::chunk(next) == Self::chunk(cell)
                    || !self
                        .async_segment_clear(Self::center(cell), Self::center(next))
                        .await
                {
                    continue;
                }
                if let Some(other) = self.async_region(next).await {
                    result.push((other, cell, next));
                }
            }
        }
        result
    }
    async fn async_connectors(&self, p: Vec2) -> Vec<IVec2> {
        let cell = Self::cell(p);
        let mut result = Vec::new();
        for y in -1..=1 {
            WorkUnit::new().await;
            for x in -1..=1 {
                WorkUnit::new().await;
                let c = cell + IVec2::new(x, y);
                if self.async_segment_clear(p, Self::center(c)).await {
                    result.push(c);
                }
            }
        }
        result
    }
    /// Reach a clear position inside a movement range without requiring access to the target center.
    async fn async_route_within_range(
        &self,
        start: Vec2,
        center: Vec2,
        radius: f32,
        grid: &PaintCosts,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        if !radius.is_finite() || radius < 0.0 || !center.is_finite() {
            return RouteOutcome::Unreachable;
        }
        if start.distance(center) <= radius && self.async_point_clear(start).await {
            return RouteOutcome::Found(Route {
                waypoints: vec![start],
                cost: 0.0,
            });
        }
        let nearest = center + (start - center).normalize_or_zero() * (radius - 0.5).max(0.0);
        if let RouteOutcome::Found(route) =
            self.async_route(start, nearest, grid, swarm, hauler).await
        {
            return RouteOutcome::Found(route);
        }
        let min = Self::cell(center - Vec2::splat(radius));
        let max = Self::cell(center + Vec2::splat(radius));
        let mut candidates = Vec::new();
        for y in min.y..=max.y {
            WorkUnit::new().await;
            for x in min.x..=max.x {
                WorkUnit::new().await;
                let p = Self::center(IVec2::new(x, y));
                if p.distance(center) <= radius && self.async_point_clear(p).await {
                    candidates.push(p);
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        for p in candidates {
            WorkUnit::new().await;
            if let RouteOutcome::Found(route) =
                self.async_route(start, p, grid, swarm, hauler).await
            {
                return RouteOutcome::Found(route);
            }
        }
        RouteOutcome::Unreachable
    }

    /// Reach any accessible exterior work position, including alternative faces.
    async fn async_route_to_interaction(
        &self,
        start: Vec2,
        region: crate::nanobot::InteractionRegion,
        grid: &PaintCosts,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        if self.async_point_clear(start).await && region.contains(start) {
            return RouteOutcome::Found(Route {
                waypoints: vec![start],
                cost: 0.0,
            });
        }
        for endpoint in region.candidates(start) {
            WorkUnit::new().await;
            if let RouteOutcome::Found(route) =
                self.async_route(start, endpoint, grid, swarm, hauler).await
            {
                return RouteOutcome::Found(route);
            }
        }
        RouteOutcome::Unreachable
    }
    async fn async_route(
        &self,
        start: Vec2,
        end: Vec2,
        grid: &PaintCosts,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        if !self.async_point_clear(start).await || !self.async_point_clear(end).await {
            return RouteOutcome::Unreachable;
        }
        if (start.distance_squared(end) < 0.0001 || !hauler)
            && self.async_segment_clear(start, end).await
        {
            return RouteOutcome::Found(Route {
                waypoints: vec![end],
                cost: start.distance(end),
            });
        }
        let direct = if self.async_segment_clear(start, end).await {
            Some(Route {
                waypoints: vec![end],
                cost: Self::paint_segment_cost(start, end, grid, swarm, hauler).await,
            })
        } else {
            None
        };
        let starts = self.async_connectors(start).await;
        let ends = self.async_connectors(end).await;
        if starts.is_empty() || ends.is_empty() {
            return direct.map_or(RouteOutcome::Unreachable, RouteOutcome::Found);
        }
        let mut goals = HashSet::new();
        for &c in &ends {
            if let Some(r) = self.async_region(c).await {
                goals.insert(r);
            }
        }
        let mut open = BinaryHeap::new();
        let mut costs = HashMap::new();
        let mut previous = HashMap::new();
        let scale = if hauler { 0.35 } else { 1.0 };
        for &c in &starts {
            WorkUnit::new().await;
            if let Some(region) = self.async_region(c).await {
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
        let mut sources = HashSet::new();
        for &c in &starts {
            if let Some(r) = self.async_region(c).await {
                sources.insert(r);
            }
        }
        let mut reverse_seen = goals.clone();
        let mut ordered_goals: Vec<_> = goals.iter().copied().collect();
        ordered_goals.sort_by_key(|p| (p.y, p.x));
        let mut reverse_queue: VecDeque<_> = ordered_goals.into();
        let mut reverse_connected = false;
        let mut reached = None;
        while let Some(Visit { node, cost, .. }) = open.pop() {
            WorkUnit::new().await;
            if cost > costs[&node] {
                continue;
            }
            self.expansions.coarse.fetch_add(1, AtomicOrdering::Relaxed);
            if goals.contains(&node) {
                reached = Some(node);
                break;
            }
            if !reverse_connected {
                let Some(reverse) = reverse_queue.pop_front() else {
                    return direct.map_or(RouteOutcome::Unreachable, RouteOutcome::Found);
                };
                self.expansions.coarse.fetch_add(1, AtomicOrdering::Relaxed);
                if sources.contains(&reverse) {
                    reverse_connected = true;
                } else {
                    for (other, _, _) in self.async_connections(reverse).await {
                        WorkUnit::new().await;
                        if reverse_seen.insert(other) {
                            reverse_queue.push_back(other);
                        }
                    }
                }
            }
            let mut neighbors: HashMap<IVec2, f32> = HashMap::new();
            for (other, cell, next) in self.async_connections(node).await {
                WorkUnit::new().await;
                let distance = Self::center(node).distance(Self::center(other));
                let weight = (Self::paint_multiplier(grid, swarm, hauler, cell)
                    + Self::paint_multiplier(grid, swarm, hauler, next))
                    * 0.5;
                neighbors
                    .entry(other)
                    .and_modify(|v| *v = v.min(distance * weight))
                    .or_insert(distance * weight);
            }
            let mut neighbors: Vec<_> = neighbors.into_iter().collect();
            neighbors.sort_by_key(|(cell, _)| (cell.y, cell.x));
            for (next, edge) in neighbors {
                WorkUnit::new().await;
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
            WorkUnit::new().await;
            allowed.insert(parent);
            node = parent;
        }
        // Every coarse edge represents an actual fine-grid edge. Restricting detail to
        // the connected regions on that chain therefore preserves reachability.
        open.clear();
        costs.clear();
        previous.clear();
        for &c in &starts {
            WorkUnit::new().await;
            if self
                .async_region(c)
                .await
                .is_some_and(|r| allowed.contains(&r))
            {
                let cost =
                    Self::paint_segment_cost(start, Self::center(c), grid, swarm, hauler).await;
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
            WorkUnit::new().await;
            if score >= best {
                break;
            }
            if cost > costs[&node] {
                continue;
            }
            self.expansions.fine.fetch_add(1, AtomicOrdering::Relaxed);
            if ends.contains(&node) {
                let total = cost
                    + Self::paint_segment_cost(Self::center(node), end, grid, swarm, hauler).await;
                if total < best {
                    best = total;
                    finish = Some(node);
                }
            }
            for d in DIRECTIONS {
                WorkUnit::new().await;
                let next = node + d;
                if !self
                    .async_region(next)
                    .await
                    .is_some_and(|r| allowed.contains(&r))
                    || !self
                        .async_segment_clear(Self::center(node), Self::center(next))
                        .await
                {
                    continue;
                }
                let weight = (Self::paint_multiplier(grid, swarm, hauler, node)
                    + Self::paint_multiplier(grid, swarm, hauler, next))
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
            WorkUnit::new().await;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteGoal {
    Point(Vec2),
    Range { center: Vec2, radius: f32 },
    Interaction(crate::nanobot::InteractionRegion),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutePriority {
    Routine,
    Invalidated,
    Clearing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RouteRequestId(u64);
#[derive(Clone, Debug)]
pub enum RouteStatus {
    Pending,
    Found(Route),
    Unreachable,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct NavigationWork {
    pub tick: u64,
    pub work: usize,
    pub pending: usize,
    pub completed: usize,
    pub max_latency_ticks: u64,
    /// Fine cells visited while materializing physical chunk connectivity.
    pub hierarchy_cells: usize,
    pub hierarchy_chunks: usize,
    /// Non-stale coarse nodes expanded, including the reverse connectivity probe.
    pub coarse_expansions: usize,
    /// Non-stale fine nodes expanded while refining the coarse route.
    pub fine_expansions: usize,
}

type Search = Pin<Box<dyn Future<Output = RouteOutcome> + Send>>;
struct Request {
    start: Vec2,
    goal: RouteGoal,
    swarm: SwarmId,
    hauler: bool,
    priority: RoutePriority,
    submitted: u64,
    revision: u64,
    future: Option<Search>,
    result: RouteStatus,
    access: Option<Arc<AccessCheck>>,
}
#[derive(Default)]
pub(super) struct Scheduler {
    next: u64,
    tick: u64,
    requests: HashMap<RouteRequestId, Request>,
    work: NavigationWork,
    paint: Arc<PaintCosts>,
    paint_revision: Option<u64>,
    cached: HashMap<String, (RouteRequestId, u64)>,
    access_cached: HashMap<(u64, u64), (RouteRequestId, u64)>,
}
#[derive(Default)]
struct PaintCosts {
    cells: HashMap<IVec2, crate::intent::IntentCell>,
}
impl PaintCosts {
    fn cell(&self, cell: IVec2) -> Option<&crate::intent::IntentCell> {
        self.cells.get(&cell)
    }
}

impl Navigation {
    pub fn request(
        &self,
        start: Vec2,
        goal: RouteGoal,
        swarm: SwarmId,
        hauler: bool,
        priority: RoutePriority,
    ) -> RouteRequestId {
        let mut scheduler = self
            .scheduler
            .lock()
            .expect("navigation scheduler poisoned");
        let id = RouteRequestId(scheduler.next);
        scheduler.next += 1;
        let tick = scheduler.tick;
        scheduler.requests.insert(
            id,
            Request {
                start,
                goal,
                swarm,
                hauler,
                priority,
                submitted: tick,
                revision: self.revision,
                future: None,
                result: RouteStatus::Pending,
                access: None,
            },
        );
        id
    }
    pub fn poll(&self, id: RouteRequestId) -> RouteStatus {
        let scheduler = self
            .scheduler
            .lock()
            .expect("navigation scheduler poisoned");
        scheduler
            .requests
            .get(&id)
            .map_or(RouteStatus::Pending, |r| {
                if r.revision != self.revision {
                    RouteStatus::Pending
                } else {
                    r.result.clone()
                }
            })
    }
    pub fn cancel(&self, id: RouteRequestId) {
        self.scheduler.lock().unwrap().requests.remove(&id);
    }
    pub fn work(&self) -> NavigationWork {
        self.scheduler.lock().unwrap().work
    }

    /// One unit resumes at most one graph edge, cell, or search iteration.
    /// Lazy hierarchy construction yields through this same allowance.
    pub fn advance(&self, grid: &IntentGrid, budget: usize) -> NavigationWork {
        let mut scheduler = self
            .scheduler
            .lock()
            .expect("navigation scheduler poisoned");
        scheduler.tick += 1;
        let tick = scheduler.tick;
        if scheduler.paint_revision != Some(grid.revision()) {
            scheduler.paint = Arc::new(PaintCosts {
                cells: grid.iter_active_cells().map(|(p, c)| (p, *c)).collect(),
            });
            scheduler.paint_revision = Some(grid.revision());
        }
        let expired: Vec<_> = scheduler
            .cached
            .iter()
            .filter(|(_, (_, touched))| tick.saturating_sub(*touched) > 120)
            .map(|(key, (id, _))| (key.clone(), *id))
            .collect();
        for (key, id) in expired {
            scheduler.cached.remove(&key);
            scheduler.requests.remove(&id);
        }
        let stale_access: Vec<_> = scheduler
            .access_cached
            .iter()
            .filter(|((_, revision), (_, touched))| {
                *revision != self.revision || tick.saturating_sub(*touched) > 120
            })
            .map(|(key, (id, _))| (*key, *id))
            .collect();
        for (key, id) in stale_access {
            scheduler.access_cached.remove(&key);
            scheduler.requests.remove(&id);
        }
        let before = self.expansions.snapshot();
        let mut work = NavigationWork {
            tick,
            ..Default::default()
        };
        for request in scheduler.requests.values_mut() {
            if request.revision != self.revision {
                request.future = None;
                request.result = RouteStatus::Pending;
                request.revision = self.revision;
            }
        }
        // Submission aging preserves eventual service without fragmenting every long
        // search across the entire pending population. Candidate order is stable for
        // this advance, so compute it once instead of scanning on every quantum.
        let mut pending: Vec<_> = scheduler
            .requests
            .iter()
            .filter(|(_, r)| matches!(r.result, RouteStatus::Pending))
            .map(|(id, r)| {
                let urgency = match r.priority {
                    RoutePriority::Routine => 0,
                    RoutePriority::Invalidated => 8,
                    RoutePriority::Clearing => 16,
                };
                (*id, tick.saturating_sub(r.submitted) + urgency)
            })
            .collect();
        pending.sort_by_key(|(id, priority)| (std::cmp::Reverse(*priority), id.0));
        for (id, _) in pending {
            if work.work >= budget {
                break;
            }
            let paint = scheduler.paint.clone();
            let request = scheduler.requests.get_mut(&id).unwrap();
            if request.future.is_none() {
                let snapshot = Navigation {
                    min: self.min,
                    max: self.max,
                    obstacles: if self.clearing.is_empty() {
                        self.obstacles.clone()
                    } else {
                        Arc::new(
                            self.obstacles
                                .iter()
                                .copied()
                                .chain(
                                    self.clearing
                                        .iter()
                                        .copied()
                                        .filter(|shape| shape.admits_body(request.start)),
                                )
                                .collect(),
                        )
                    },
                    clearing: Vec::new(),
                    revision: self.revision,
                    chunks: if self.clearing.is_empty() {
                        self.chunks.clone()
                    } else {
                        Arc::new(Mutex::new(HashMap::new()))
                    },
                    scheduler: Mutex::new(Scheduler::default()),
                    expansions: self.expansions.clone(),
                    chunk_builds: if self.clearing.is_empty() {
                        self.chunk_builds.clone()
                    } else {
                        Default::default()
                    },
                };
                let (start, goal, swarm, hauler) =
                    (request.start, request.goal, request.swarm, request.hauler);
                let access = request.access.clone();
                request.future = Some(Box::pin(async move {
                    if let Some(check) = access {
                        return snapshot.async_access(&check, &paint).await;
                    }
                    match goal {
                        RouteGoal::Point(end) => {
                            snapshot
                                .async_route(start, end, &paint, swarm, hauler)
                                .await
                        }
                        RouteGoal::Range { center, radius } => {
                            snapshot
                                .async_route_within_range(
                                    start, center, radius, &paint, swarm, hauler,
                                )
                                .await
                        }
                        RouteGoal::Interaction(region) => {
                            snapshot
                                .async_route_to_interaction(start, region, &paint, swarm, hauler)
                                .await
                        }
                    }
                }));
            }
            let mut context = Context::from_waker(Waker::noop());
            let quantum = budget - work.work;
            for _ in 0..quantum {
                work.work += 1;
                if let Poll::Ready(result) =
                    request.future.as_mut().unwrap().as_mut().poll(&mut context)
                {
                    request.result = match result {
                        RouteOutcome::Found(route) => RouteStatus::Found(route),
                        RouteOutcome::Unreachable => RouteStatus::Unreachable,
                    };
                    request.future = None;
                    work.completed += 1;
                    work.max_latency_ticks = work.max_latency_ticks.max(tick - request.submitted);
                    break;
                }
            }
        }
        work.pending = scheduler
            .requests
            .values()
            .filter(|r| matches!(r.result, RouteStatus::Pending))
            .count();
        let after = self.expansions.snapshot();
        work.hierarchy_cells = after.hierarchy_cells - before.hierarchy_cells;
        work.coarse_expansions = after.coarse_expansions - before.coarse_expansions;
        work.fine_expansions = after.fine_expansions - before.fine_expansions;
        work.hierarchy_chunks = after.hierarchy_chunks - before.hierarchy_chunks;
        scheduler.work = work;
        work
    }
}

impl Navigation {
    fn paint_multiplier(grid: &PaintCosts, swarm: SwarmId, hauler: bool, cell: IVec2) -> f32 {
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
    async fn paint_segment_cost(
        start: Vec2,
        end: Vec2,
        grid: &PaintCosts,
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
                WorkUnit::new().await;
                splits.push((edge as f32 * CELL_WIDTH - start[axis]) / delta[axis]);
            }
        }
        splits.sort_by(f32::total_cmp);
        let mut cost = 0.0;
        for pair in splits.windows(2) {
            WorkUnit::new().await;
            let midpoint = start + delta * ((pair[0] + pair[1]) * 0.5);
            cost += (pair[1] - pair[0])
                * delta.length()
                * Self::paint_multiplier(grid, swarm, hauler, Self::cell(midpoint));
        }
        cost
    }
}

impl Navigation {
    pub(super) fn solve(
        &self,
        start: Vec2,
        goal: RouteGoal,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        let paint = PaintCosts {
            cells: grid.iter_active_cells().map(|(p, c)| (p, *c)).collect(),
        };
        let future = async {
            match goal {
                RouteGoal::Point(end) => self.async_route(start, end, &paint, swarm, hauler).await,
                RouteGoal::Range { center, radius } => {
                    self.async_route_within_range(start, center, radius, &paint, swarm, hauler)
                        .await
                }
                RouteGoal::Interaction(region) => {
                    self.async_route_to_interaction(start, region, &paint, swarm, hauler)
                        .await
                }
            }
        };
        let mut future = std::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
                return result;
            }
        }
    }
}

impl Navigation {
    pub fn refresh_clearing(&mut self, clearing: Vec<Obstacle>) {
        if self.clearing != clearing {
            self.clearing = clearing;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

impl Navigation {
    /// Repeated allocation probes share a request; pending probes do not establish no-route.
    pub fn query(
        &self,
        start: Vec2,
        goal: RouteGoal,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteStatus {
        let cell = Self::cell(start);
        let key = format!("{cell:?}:{goal:?}:{swarm:?}:{hauler}:{}", grid.revision());
        let mut scheduler = self.scheduler.lock().unwrap();
        let tick = scheduler.tick;
        if let Some(&(id, _)) = scheduler.cached.get(&key) {
            let reusable = scheduler
                .requests
                .get(&id)
                .is_some_and(|r| self.segment_clear(start, r.start));
            if reusable {
                scheduler.cached.get_mut(&key).unwrap().1 = tick;
                drop(scheduler);
                return self.poll(id);
            }
            scheduler.cached.remove(&key);
            scheduler.requests.remove(&id);
        }
        drop(scheduler);
        let id = self.request(start, goal, swarm, hauler, RoutePriority::Routine);
        self.scheduler
            .lock()
            .unwrap()
            .cached
            .insert(key, (id, tick));
        RouteStatus::Pending
    }
    pub fn query_interaction(
        &self,
        start: Vec2,
        region: crate::nanobot::InteractionRegion,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteStatus {
        self.query(start, RouteGoal::Interaction(region), grid, swarm, hauler)
    }
    pub fn query_point(
        &self,
        start: Vec2,
        end: Vec2,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteStatus {
        self.query(start, RouteGoal::Point(end), grid, swarm, hauler)
    }
}

impl Navigation {
    async fn async_point_clear(&self, p: Vec2) -> bool {
        if !p.is_finite() || !p.cmpge(self.min).all() || !p.cmplt(self.max).all() {
            return false;
        }
        for obstacle in self.obstacles.iter() {
            WorkUnit::new().await;
            if !obstacle.admits_body(p) {
                return false;
            }
        }
        true
    }
    async fn async_segment_clear(&self, a: Vec2, b: Vec2) -> bool {
        if !a.is_finite()
            || !b.is_finite()
            || !a.cmpge(self.min).all()
            || !a.cmplt(self.max).all()
            || !b.cmpge(self.min).all()
            || !b.cmplt(self.max).all()
        {
            return false;
        }
        for obstacle in self.obstacles.iter() {
            WorkUnit::new().await;
            if !obstacle.segment_clear(a, b) {
                return false;
            }
        }
        true
    }
}

/// A hypothetical completion checked by the same budget as movement requests.
pub struct AccessCheck {
    pub baseline: Vec<Obstacle>,
    pub completed: Vec<Obstacle>,
    pub builders: Vec<Vec2>,
    pub target: crate::nanobot::InteractionRegion,
    pub endpoints: Vec<crate::nanobot::InteractionRegion>,
    pub swarm: SwarmId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessStatus {
    Pending,
    Accepted,
    Rejected,
}
impl Navigation {
    pub fn query_access(
        &self,
        key: u64,
        builders: &[Vec2],
        build: impl FnOnce() -> AccessCheck,
        priority: RoutePriority,
    ) -> AccessStatus {
        let cache_key = (key, self.revision);
        let mut scheduler = self.scheduler.lock().unwrap();
        let tick = scheduler.tick;
        if let Some(&(id, _)) = scheduler.access_cached.get(&cache_key) {
            let compatible = scheduler
                .requests
                .get(&id)
                .and_then(|r| r.access.as_ref())
                .is_some_and(|check| {
                    let linked = |a: Vec2, b: Vec2| {
                        a == b
                            || (a.is_finite()
                                && b.is_finite()
                                && a.cmpge(self.min).all()
                                && a.cmplt(self.max).all()
                                && b.cmpge(self.min).all()
                                && b.cmplt(self.max).all()
                                && check
                                    .completed
                                    .iter()
                                    .all(|shape| shape.segment_clear(a, b)))
                    };
                    check.builders.len() == builders.len()
                        && check
                            .builders
                            .iter()
                            .all(|&old| builders.iter().any(|&new| linked(old, new)))
                        && builders
                            .iter()
                            .all(|&new| check.builders.iter().any(|&old| linked(old, new)))
                });
            if compatible {
                scheduler.access_cached.get_mut(&cache_key).unwrap().1 = tick;
                drop(scheduler);
                return match self.poll(id) {
                    RouteStatus::Pending => AccessStatus::Pending,
                    RouteStatus::Found(_) => AccessStatus::Accepted,
                    RouteStatus::Unreachable => AccessStatus::Rejected,
                };
            }
            scheduler.access_cached.remove(&cache_key);
            scheduler.requests.remove(&id);
        }
        drop(scheduler);
        let check = Arc::new(build());
        let id = self.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::ZERO),
            check.swarm,
            false,
            priority,
        );
        let mut scheduler = self.scheduler.lock().unwrap();
        scheduler.requests.get_mut(&id).unwrap().access = Some(check);
        scheduler.access_cached.insert(cache_key, (id, tick));
        AccessStatus::Pending
    }
    fn hypothetical(&self, obstacles: Vec<Obstacle>) -> Navigation {
        Navigation {
            min: self.min,
            max: self.max,
            obstacles: Arc::new(obstacles),
            clearing: Vec::new(),
            revision: self.revision,
            chunks: Arc::new(Mutex::new(HashMap::new())),
            scheduler: Mutex::new(Scheduler::default()),
            expansions: self.expansions.clone(),
            chunk_builds: Default::default(),
        }
    }
    async fn async_connected(
        &self,
        a: crate::nanobot::InteractionRegion,
        b: crate::nanobot::InteractionRegion,
        paint: &PaintCosts,
        swarm: SwarmId,
    ) -> bool {
        for point in a.candidates(Vec2::ZERO) {
            WorkUnit::new().await;
            if matches!(
                self.async_route_to_interaction(point, b, paint, swarm, false)
                    .await,
                RouteOutcome::Found(_)
            ) {
                return true;
            }
        }
        false
    }
    async fn async_access(&self, check: &AccessCheck, paint: &PaintCosts) -> RouteOutcome {
        WorkUnit::new().await;
        let baseline = self.hypothetical(check.baseline.clone());
        let completed = self.hypothetical(check.completed.clone());
        let mut builder_reaches = false;
        for &builder in &check.builders {
            WorkUnit::new().await;
            if matches!(
                completed
                    .async_route_to_interaction(builder, check.target, paint, check.swarm, false)
                    .await,
                RouteOutcome::Found(_)
            ) {
                builder_reaches = true;
                break;
            }
        }
        if !builder_reaches {
            return RouteOutcome::Unreachable;
        }
        for (index, &a) in check.endpoints.iter().enumerate() {
            WorkUnit::new().await;
            for &b in &check.endpoints[index + 1..] {
                WorkUnit::new().await;
                if baseline.async_connected(a, b, paint, check.swarm).await
                    && !completed.async_connected(a, b, paint, check.swarm).await
                {
                    return RouteOutcome::Unreachable;
                }
            }
        }
        RouteOutcome::Found(Route {
            waypoints: Vec::new(),
            cost: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypothetical_construction_and_movement_share_the_same_allowance() {
        let grid = IntentGrid::new(4, 4);
        let navigation = Navigation::new(&grid, vec![]);
        let candidate = Transform::from_xyz(144.0, 0.0, 0.0);
        let builders = [Vec2::ZERO];
        let check = || AccessCheck {
            baseline: vec![],
            completed: vec![Obstacle::structure(&candidate)],
            builders: builders.to_vec(),
            target: crate::nanobot::InteractionRegion::structure(&candidate),
            endpoints: vec![],
            swarm: SwarmId::PLAYER,
        };
        assert_eq!(
            navigation.query_access(41, &builders, check, RoutePriority::Routine),
            AccessStatus::Pending
        );
        let movement = navigation.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::splat(100.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Clearing,
        );
        assert_eq!(navigation.advance(&grid, 0).work, 0);
        assert_eq!(
            navigation.query_access(41, &builders, check, RoutePriority::Routine),
            AccessStatus::Pending
        );
        assert_eq!(navigation.advance(&grid, 1).work, 1);
        assert!(matches!(navigation.poll(movement), RouteStatus::Found(_)));
        assert_eq!(
            navigation.query_access(41, &builders, check, RoutePriority::Routine),
            AccessStatus::Pending
        );
        let mut result = AccessStatus::Pending;
        for _ in 0..100 {
            assert!(navigation.advance(&grid, 3).work <= 3);
            result = navigation.query_access(41, &builders, check, RoutePriority::Routine);
            if result != AccessStatus::Pending {
                break;
            }
        }
        assert_eq!(result, AccessStatus::Accepted);
    }

    #[test]
    fn clearing_precedes_routine_but_aged_routine_is_eventually_served() {
        let grid = IntentGrid::new(4, 4);
        let navigation = Navigation::new(&grid, vec![]);
        let routine = navigation.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::splat(100.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        let invalidated = navigation.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::splat(100.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Invalidated,
        );
        let urgent = navigation.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::splat(100.0)),
            SwarmId(7),
            false,
            RoutePriority::Clearing,
        );
        assert_eq!(navigation.advance(&grid, 1).work, 1);
        assert!(matches!(navigation.poll(urgent), RouteStatus::Found(_)));
        assert!(matches!(navigation.poll(routine), RouteStatus::Pending));
        assert!(matches!(navigation.poll(invalidated), RouteStatus::Pending));
        navigation.advance(&grid, 1);
        assert!(matches!(
            navigation.poll(invalidated),
            RouteStatus::Found(_)
        ));
        assert!(matches!(navigation.poll(routine), RouteStatus::Pending));
        for _ in 0..20 {
            navigation.request(
                Vec2::ZERO,
                RouteGoal::Point(Vec2::splat(100.0)),
                SwarmId(7),
                false,
                RoutePriority::Clearing,
            );
            navigation.advance(&grid, 1);
        }
        assert!(
            matches!(navigation.poll(routine), RouteStatus::Found(_)),
            "ongoing urgent arrivals starved an older routine request"
        );
    }

    #[test]
    fn changed_obstacles_reject_finished_results_and_reopen_unreachable_requests() {
        let grid = IntentGrid::new(4, 4);
        let mut navigation = Navigation::new(&grid, vec![]);
        let request = navigation.request(
            Vec2::new(-100.0, 0.0),
            RouteGoal::Point(Vec2::new(100.0, 0.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Invalidated,
        );
        navigation.advance(&grid, 1);
        assert!(matches!(navigation.poll(request), RouteStatus::Found(_)));
        navigation.refresh(
            &grid,
            vec![Obstacle::Circle {
                center: Vec2::new(100.0, 0.0),
                radius: 40.0,
            }],
        );
        assert!(
            matches!(navigation.poll(request), RouteStatus::Pending),
            "a stale route entered a new solid destination"
        );
        navigation.advance(&grid, 8);
        assert!(matches!(navigation.poll(request), RouteStatus::Unreachable));
        navigation.refresh(&grid, vec![]);
        assert!(matches!(navigation.poll(request), RouteStatus::Pending));
        navigation.advance(&grid, 1);
        assert!(matches!(navigation.poll(request), RouteStatus::Found(_)));
    }

    #[test]
    fn repeated_pending_queries_share_work_and_keep_their_result() {
        let grid = IntentGrid::new(4, 4);
        let navigation = Navigation::new(&grid, vec![]);
        for _ in 0..100 {
            assert!(matches!(
                navigation.query_point(
                    Vec2::ZERO,
                    Vec2::splat(100.0),
                    &grid,
                    SwarmId::PLAYER,
                    false
                ),
                RouteStatus::Pending
            ));
        }
        let work = navigation.advance(&grid, 1);
        assert_eq!(work.completed, 1);
        assert_eq!(work.pending, 0);
        assert!(matches!(
            navigation.query_point(
                Vec2::ZERO,
                Vec2::splat(100.0),
                &grid,
                SwarmId::PLAYER,
                false
            ),
            RouteStatus::Found(_)
        ));
    }

    #[test]
    fn simultaneous_replanning_shares_one_tick_allowance() {
        let grid = IntentGrid::new(4, 4);
        let mut navigation = Navigation::new(&grid, vec![]);
        let requests: Vec<_> = (0..24)
            .map(|_| {
                navigation.request(
                    Vec2::new(-252.0, 36.0),
                    RouteGoal::Point(Vec2::new(252.0, 36.0)),
                    SwarmId::PLAYER,
                    false,
                    RoutePriority::Invalidated,
                )
            })
            .collect();
        navigation.advance(&grid, 1000);
        navigation.refresh(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(72.0, 144.0),
            }],
        );
        let mut completed = 0;
        let mut ticks = 0;
        let mut max_latency = 0;
        while completed < requests.len() && ticks < 1000 {
            let work = navigation.advance(&grid, 1000);
            assert!(work.work <= 1000);
            completed += work.completed;
            max_latency = max_latency.max(work.max_latency_ticks);
            ticks += 1;
        }
        assert_eq!(completed, 24);
        for request in requests {
            assert!(matches!(navigation.poll(request), RouteStatus::Found(_)));
        }
        eprintln!(
            "24 simultaneous replans: {ticks} ticks, max latency {max_latency}, cap 1000 units including lazy hierarchy maintenance"
        );
    }

    #[test]
    fn a_detour_stays_pending_until_its_bounded_work_finishes() {
        let grid = IntentGrid::new(4, 4);
        let navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(72.0, 144.0),
            }],
        );
        let request = navigation.request(
            Vec2::new(-252.0, 36.0),
            RouteGoal::Point(Vec2::new(252.0, 36.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        assert!(matches!(navigation.poll(request), RouteStatus::Pending));
        assert_eq!(navigation.advance(&grid, 0).work, 0);
        assert!(matches!(navigation.poll(request), RouteStatus::Pending));
        assert_eq!(navigation.advance(&grid, 3).work, 3);
        assert!(matches!(navigation.poll(request), RouteStatus::Pending));
        let mut ticks = 0;
        while matches!(navigation.poll(request), RouteStatus::Pending) && ticks < 2000 {
            assert!(navigation.advance(&grid, 100).work <= 100);
            ticks += 1;
        }
        let RouteStatus::Found(route) = navigation.poll(request) else {
            panic!("detour did not finish after {ticks} ticks")
        };
        assert!(route.cost > 504.0);
        assert_eq!(route.waypoints.last(), Some(&Vec2::new(252.0, 36.0)));
        let mut previous = Vec2::new(-252.0, 36.0);
        for point in route.waypoints {
            assert!(navigation.segment_clear(previous, point));
            previous = point;
        }
        eprintln!("detour latency {ticks} ticks at 100 units/tick");
    }
}
