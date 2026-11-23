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
    cell::Cell,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

pub(super) type ChunkBuild = Pin<Box<dyn Future<Output = Chunk> + Send>>;

pub(super) struct WorkUnit(bool);
impl WorkUnit {
    pub(super) fn new() -> Self {
        Self(false)
    }
}
impl Future for WorkUnit {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.0 || take_allowance() {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

thread_local! {
    /// Further work units the current [`poll_units`] call may run without yielding.
    static ALLOWANCE: Cell<usize> = const { Cell::new(0) };
}

fn take_allowance() -> bool {
    ALLOWANCE.with(|allowance| {
        let left = allowance.get();
        allowance.set(left.saturating_sub(1));
        left > 0
    })
}

/// Restores the enclosing allowance when a poll scope ends.
struct AllowanceScope(usize);
impl AllowanceScope {
    fn enter(allowance: usize) -> Self {
        Self(ALLOWANCE.replace(allowance))
    }
}
impl Drop for AllowanceScope {
    fn drop(&mut self) {
        ALLOWANCE.set(self.0);
    }
}

/// Polls `future` for at most `units` work units in one call, with the same
/// result as polling it once per unit and stopping at the first ready poll.
/// Returns the poll result and the units used.
pub(super) fn poll_units<F: Future + ?Sized>(
    future: Pin<&mut F>,
    units: usize,
) -> (Poll<F::Output>, usize) {
    debug_assert!(units > 0);
    let scope = AllowanceScope::enter(units - 1);
    let poll = future.poll(&mut Context::from_waker(Waker::noop()));
    let used = units - ALLOWANCE.get();
    drop(scope);
    debug_assert!(poll.is_ready() || used == units);
    (poll, used)
}

/// Polls until ready, one work unit per poll while `allow_work` permits.
pub(super) fn poll_with_budget<F: Future>(
    future: F,
    allow_work: &mut dyn FnMut() -> bool,
) -> Result<F::Output, ()> {
    // A nested synchronous poll meters its own units, not the enclosing request's.
    let _scope = AllowanceScope::enter(0);
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    while allow_work() {
        if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return Ok(result);
        }
    }
    Err(())
}

impl Navigation {
    fn region_build_snapshot(&self) -> Navigation {
        Navigation {
            min: self.min,
            max: self.max,
            obstacles: self.obstacles.clone(),
            obstacle_index: self.obstacle_index.clone(),
            clearing: Vec::new(),
            clearing_layer: None,
            base: None,
            revision: self.revision,
            clearance: super::next_clearance(),
            chunks: Arc::new(Mutex::new(FastHashMap::default())),
            connections: Default::default(),
            scheduler: Mutex::new(Scheduler::default()),
            chunk_builds: Default::default(),
            expansions: self.expansions.clone(),
        }
    }

    async fn async_regions(&self, chunk: IVec2) -> Chunk {
        self.region_owner(chunk).async_own_regions(chunk).await
    }

    async fn async_own_regions(&self, chunk: IVec2) -> Chunk {
        std::future::poll_fn(|context| {
            if let Some(cached) = self.chunks.lock().unwrap().get(&chunk).cloned() {
                return Poll::Ready(cached);
            }
            let mut builds = self.chunk_builds.lock().unwrap();
            let build = builds.entry(chunk).or_insert_with(|| {
                // The build snapshot owns independent empty caches, so retaining a
                // partial build cannot form an Arc cycle back to this shared cache.
                let snapshot = self.region_build_snapshot();
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
        let mut regions = FastHashMap::default();
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
        let owner = self.region_owner(key);
        if let Some(chunk) = owner
            .chunks
            .lock()
            .expect("navigation chunk cache poisoned")
            .get(&key)
        {
            return chunk.regions.get(&cell).copied();
        }
        owner
            .async_own_regions(key)
            .await
            .regions
            .get(&cell)
            .copied()
    }
    async fn async_connections(&self, node: IVec2) -> Vec<(IVec2, IVec2, IVec2)> {
        self.edge_owner(Self::chunk(node))
            .async_own_connections(node)
            .await
    }
    async fn async_own_connections(&self, node: IVec2) -> Vec<(IVec2, IVec2, IVec2)> {
        if let Some(edges) = self.connections.lock().unwrap().get(&node).cloned() {
            return edges;
        }
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
        self.connections
            .lock()
            .unwrap()
            .insert(node, result.clone());
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
        meter_probes: bool,
    ) -> RouteOutcome {
        let start_clear = self.async_point_clear(start).await;
        if start_clear && region.contains(start) {
            return RouteOutcome::Found(Route {
                waypoints: vec![start],
                cost: 0.0,
            });
        }
        let mut probes = self
            .interaction_probes(start, region, start_clear, meter_probes)
            .await;
        loop {
            WorkUnit::new().await;
            let Some(probe) = probes.next() else {
                break;
            };
            let task = self.resolve_interaction_probe(start, probe);
            let endpoint = if meter_probes {
                task.await
            } else {
                poll_with_budget(task, &mut || true).expect("unlimited interaction probe")
            };
            let Some(endpoint) = endpoint else {
                continue;
            };
            if let RouteOutcome::Found(route) =
                self.async_route(start, endpoint, grid, swarm, hauler).await
            {
                return RouteOutcome::Found(route);
            }
        }
        RouteOutcome::Unreachable
    }
    async fn async_connectivity(&self, start: Vec2, goal: RouteGoal) -> RouteOutcome {
        if !self.async_point_clear(start).await {
            return RouteOutcome::Unreachable;
        }
        match goal {
            RouteGoal::Point(end) => self.first_connected_endpoint(start, [Some(end)]).await,
            RouteGoal::Interaction(region) => {
                if region.contains(start) {
                    return RouteOutcome::Found(Route {
                        waypoints: vec![start],
                        cost: 0.0,
                    });
                }
                let probes = self
                    .interaction_probes(start, region, true, false)
                    .await
                    .map(|probe| {
                        let task = self.resolve_interaction_probe(start, probe);
                        poll_with_budget(task, &mut || true).expect("unlimited interaction probe")
                    });
                self.first_connected_endpoint(start, probes).await
            }
            RouteGoal::Range { center, radius } => {
                if !radius.is_finite() || radius < 0.0 || !center.is_finite() {
                    return RouteOutcome::Unreachable;
                }
                if start.distance(center) <= radius {
                    return RouteOutcome::Found(Route {
                        waypoints: vec![start],
                        cost: 0.0,
                    });
                }
                let mut points =
                    vec![center + (start - center).normalize_or_zero() * (radius - 0.5).max(0.0)];
                let min = Self::cell(center - Vec2::splat(radius));
                let max = Self::cell(center + Vec2::splat(radius));
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        WorkUnit::new().await;
                        let point = Self::center(IVec2::new(x, y));
                        if point.distance(center) <= radius {
                            points.push(point);
                        }
                    }
                }
                points.sort_by(|a, b| {
                    a.distance_squared(start)
                        .total_cmp(&b.distance_squared(start))
                });
                self.first_connected_endpoint(start, points.into_iter().map(Some))
                    .await
            }
        }
    }

    async fn first_connected_endpoint(
        &self,
        start: Vec2,
        probes: impl IntoIterator<Item = Option<Vec2>>,
    ) -> RouteOutcome {
        let paint = PaintCosts::default();
        let mut probes = probes.into_iter();
        loop {
            WorkUnit::new().await;
            let Some(probe) = probes.next() else {
                break;
            };
            let Some(endpoint) = probe else {
                continue;
            };
            if let result @ RouteOutcome::Found(_) = self
                .async_route_search(start, endpoint, &paint, SwarmId::PLAYER, false, true)
                .await
            {
                return result;
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
        self.async_route_search(start, end, grid, swarm, hauler, false)
            .await
    }
    async fn async_route_search(
        &self,
        start: Vec2,
        end: Vec2,
        grid: &PaintCosts,
        swarm: SwarmId,
        hauler: bool,
        connectivity_only: bool,
    ) -> RouteOutcome {
        if !self.async_point_clear(start).await || !self.async_point_clear(end).await {
            return RouteOutcome::Unreachable;
        }
        if (start.distance_squared(end) < 0.0001
            || !hauler
            || !grid.corridor_swarms.contains(&swarm))
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
        let scale = if hauler {
            grid.corridor_cost_multiplier
        } else {
            1.0
        };
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
        if connectivity_only {
            return RouteOutcome::Found(Route {
                waypoints: vec![end],
                cost: start.distance(end),
            });
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
/// Geometric access only; a connected endpoint is not a movement route.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectivityStatus {
    Pending,
    Connected { endpoint: Vec2 },
    Unreachable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestPurpose {
    Movement,
    Connectivity,
    Construction,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct NavigationWork {
    pub tick: u64,
    pub work: usize,
    pub pending: usize,
    pub completed: usize,
    pub movement_work: usize,
    pub background_work: usize,
    pub movement_pending: usize,
    pub owned_movement_pending: usize,
    pub connectivity_pending: usize,
    pub construction_pending: usize,
    pub oldest_movement_ticks: u64,
    pub oldest_background_ticks: u64,
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
    purpose: RequestPurpose,
    owner: Option<Entity>,
    last_served: u64,
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
    corridor_cost_multiplier: Option<f32>,
    access_cached: HashMap<(u64, u64), (RouteRequestId, u64)>,
    connectivity_cached: HashMap<ConnectivityKey, (RouteRequestId, u64)>,
}
/// A connectivity answer is shared by origins in one exact component, or else by one cell.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ConnectivityKey {
    revision: u64,
    origin: ConnectivityOrigin,
    goal: GoalKey,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ConnectivityOrigin {
    Region(IVec2),
    Cell(IVec2),
}
/// Exact bit identity of a goal.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GoalKey {
    Point([u32; 2]),
    Range([u32; 3]),
    Interaction(crate::nanobot::InteractionBits),
}
impl From<RouteGoal> for GoalKey {
    fn from(goal: RouteGoal) -> Self {
        match goal {
            RouteGoal::Point(point) => Self::Point(point.to_array().map(f32::to_bits)),
            RouteGoal::Range { center, radius } => {
                Self::Range([center.x.to_bits(), center.y.to_bits(), radius.to_bits()])
            }
            RouteGoal::Interaction(region) => Self::Interaction(region.bits()),
        }
    }
}
/// Untouched cache entries expire after this many ticks.
const STALE_TICKS: u64 = 120;
/// Expiring entries release their requests over several ticks, oldest first.
const PRUNE_LIMIT: usize = 64;
enum CacheEntry {
    Access((u64, u64)),
    Connectivity(ConnectivityKey),
}
#[derive(Default)]
struct PaintCosts {
    corridor_cost_multiplier: f32,
    corridor_swarms: HashSet<SwarmId>,
    cells: HashMap<IVec2, crate::intent::IntentCell>,
}
impl PaintCosts {
    fn cell(&self, cell: IVec2) -> Option<&crate::intent::IntentCell> {
        self.cells.get(&cell)
    }
}

impl Navigation {
    pub fn set_corridor_cost_multiplier(&mut self, multiplier: f32) {
        let mut scheduler = self
            .scheduler
            .lock()
            .expect("navigation scheduler poisoned");
        if scheduler.corridor_cost_multiplier == Some(multiplier) {
            return;
        }
        scheduler.corridor_cost_multiplier = Some(multiplier);
        scheduler.paint_revision = None;
        self.revision = self.revision.wrapping_add(1);
    }

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
                purpose: RequestPurpose::Movement,
                owner: None,
                last_served: tick,
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
    /// A committed route with diagnostic ownership and explicit cancellation by its caller.
    pub fn request_owned(
        &self,
        start: Vec2,
        goal: RouteGoal,
        swarm: SwarmId,
        hauler: bool,
        priority: RoutePriority,
        owner: Entity,
    ) -> RouteRequestId {
        let id = self.request(start, goal, swarm, hauler, priority);
        self.scheduler
            .lock()
            .unwrap()
            .requests
            .get_mut(&id)
            .unwrap()
            .owner = Some(owner);
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

    /// Oldest unfinished committed request belonging to this bot, in simulation ticks.
    pub fn pending_movement_ticks(&self, owner: Entity) -> Option<u64> {
        let scheduler = self.scheduler.lock().unwrap();
        scheduler
            .requests
            .values()
            .filter(|request| {
                request.purpose == RequestPurpose::Movement
                    && request.owner == Some(owner)
                    && (request.revision != self.revision
                        || matches!(request.result, RouteStatus::Pending))
            })
            .map(|request| scheduler.tick.saturating_sub(request.submitted))
            .max()
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
                corridor_cost_multiplier: scheduler.corridor_cost_multiplier.unwrap_or_else(|| {
                    crate::tuning::Tuning::default()
                        .logistics
                        .corridor_cost_multiplier
                }),
                corridor_swarms: grid
                    .iter_active_cells()
                    .flat_map(|(_, cell)| cell.owners(IntentKind::Corridor))
                    .collect(),
                cells: grid
                    .iter_active_cells()
                    .map(|(p, c)| (p, c.clone()))
                    .collect(),
            });
            scheduler.paint_revision = Some(grid.revision());
        }
        let expiring = self.prune_stale(&mut scheduler, tick);
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
        let mut lanes = [Vec::new(), Vec::new()];
        for (&id, request) in &scheduler.requests {
            if !matches!(request.result, RouteStatus::Pending) || expiring.contains(&id) {
                continue;
            }
            let boost = match request.priority {
                RoutePriority::Routine => 0,
                RoutePriority::Invalidated => 8,
                RoutePriority::Clearing => 16,
            };
            let lane = usize::from(request.purpose != RequestPurpose::Movement);
            lanes[lane].push((id, tick.saturating_sub(request.last_served) + boost));
        }
        for lane in &mut lanes {
            lane.sort_by_key(|(id, urgency)| (std::cmp::Reverse(*urgency), id.0));
        }
        let mut queues =
            lanes.map(|lane| lane.into_iter().map(|(id, _)| id).collect::<VecDeque<_>>());
        // Three quarters of capacity serves committed motion. Fractional shares rotate
        // across ticks so even a one-unit allowance eventually serves both classes.
        let background = budget / 4 + usize::from(tick % 4 < (budget % 4) as u64);
        let shares = [budget - background, background];
        for lane in 0..2 {
            self.advance_lane(&mut scheduler, &mut queues[lane], shares[lane], &mut work);
        }
        for queue in &mut queues {
            let unused = budget - work.work;
            self.advance_lane(&mut scheduler, queue, unused, &mut work);
        }
        for (id, request) in &scheduler.requests {
            if !matches!(request.result, RouteStatus::Pending) || expiring.contains(id) {
                continue;
            }
            work.pending += 1;
            let age = tick.saturating_sub(request.submitted);
            match request.purpose {
                RequestPurpose::Movement => {
                    work.movement_pending += 1;
                    work.owned_movement_pending += usize::from(request.owner.is_some());
                    work.oldest_movement_ticks = work.oldest_movement_ticks.max(age);
                }
                purpose => {
                    work.oldest_background_ticks = work.oldest_background_ticks.max(age);
                    match purpose {
                        RequestPurpose::Connectivity => work.connectivity_pending += 1,
                        RequestPurpose::Construction => work.construction_pending += 1,
                        RequestPurpose::Movement => unreachable!(),
                    }
                }
            }
        }
        let after = self.expansions.snapshot();
        work.hierarchy_cells = after.hierarchy_cells - before.hierarchy_cells;
        work.coarse_expansions = after.coarse_expansions - before.coarse_expansions;
        work.fine_expansions = after.fine_expansions - before.fine_expansions;
        work.hierarchy_chunks = after.hierarchy_chunks - before.hierarchy_chunks;
        scheduler.work = work;
        work
    }
    /// Expire stale cache entries, at most `PRUNE_LIMIT` per tick and oldest first.
    /// Returns requests still awaiting expiry, which receive no further work.
    fn prune_stale(&self, scheduler: &mut Scheduler, tick: u64) -> HashSet<RouteRequestId> {
        let stale_age = |touched: u64| tick.saturating_sub(touched) > STALE_TICKS;
        let mut stale: Vec<_> = scheduler
            .access_cached
            .iter()
            .filter(|&(&(_, revision), &(_, touched))| {
                revision != self.revision || stale_age(touched)
            })
            .map(|(&key, &(id, touched))| (touched, id, CacheEntry::Access(key)))
            .collect();
        stale.extend(
            scheduler
                .connectivity_cached
                .iter()
                .filter(|&(key, &(id, touched))| {
                    key.revision != self.revision
                        || stale_age(touched)
                        || !scheduler.requests.contains_key(&id)
                })
                .map(|(&key, &(id, touched))| (touched, id, CacheEntry::Connectivity(key))),
        );
        if stale.len() > PRUNE_LIMIT {
            stale.select_nth_unstable_by_key(PRUNE_LIMIT, |&(touched, id, _)| (touched, id.0));
        }
        let waiting = stale.split_off(stale.len().min(PRUNE_LIMIT));
        for (_, id, entry) in stale {
            match entry {
                CacheEntry::Access(key) => scheduler.access_cached.remove(&key),
                CacheEntry::Connectivity(key) => scheduler.connectivity_cached.remove(&key),
            };
            scheduler.requests.remove(&id);
        }
        waiting.into_iter().map(|(_, id, _)| id).collect()
    }
    fn advance_lane(
        &self,
        scheduler: &mut Scheduler,
        queue: &mut VecDeque<RouteRequestId>,
        allowance: usize,
        work: &mut NavigationWork,
    ) {
        const QUANTUM: usize = 128;
        let stop = work.work + allowance;
        while work.work < stop {
            let Some(id) = queue.pop_front() else {
                break;
            };
            let paint = scheduler.paint.clone();
            let request = scheduler.requests.get_mut(&id).unwrap();
            if request.future.is_none() {
                // Hypothetical construction geometry derives from the main hierarchy.
                let snapshot = if request.access.is_some() {
                    Arc::new(self.shared_snapshot())
                } else {
                    self.request_geometry(request.start)
                };
                let (start, goal, swarm, hauler) =
                    (request.start, request.goal, request.swarm, request.hauler);
                let access = request.access.clone();
                let connectivity = request.purpose == RequestPurpose::Connectivity;
                request.future = Some(Box::pin(async move {
                    if let Some(check) = access {
                        return snapshot.async_access(&check, &paint).await;
                    }
                    if connectivity {
                        return snapshot.async_connectivity(start, goal).await;
                    }
                    snapshot
                        .async_solve(start, goal, &paint, swarm, hauler, false)
                        .await
                }));
            }
            request.last_served = scheduler.tick;
            let (poll, used) = poll_units(
                request.future.as_mut().unwrap().as_mut(),
                QUANTUM.min(stop - work.work),
            );
            work.work += used;
            if request.purpose == RequestPurpose::Movement {
                work.movement_work += used;
            } else {
                work.background_work += used;
            }
            if let Poll::Ready(result) = poll {
                request.result = match result {
                    RouteOutcome::Found(route) => RouteStatus::Found(route),
                    RouteOutcome::Unreachable => RouteStatus::Unreachable,
                };
                request.future = None;
                work.completed += 1;
                work.max_latency_ticks = work
                    .max_latency_ticks
                    .max(scheduler.tick - request.submitted);
            }
            if matches!(request.result, RouteStatus::Pending) {
                queue.push_back(id);
            }
        }
    }
}

impl Navigation {
    fn paint_multiplier(grid: &PaintCosts, swarm: SwarmId, hauler: bool, cell: IVec2) -> f32 {
        if hauler
            && grid
                .cell(world_to_cell(Self::center(cell)))
                .is_some_and(|c| c.has_owned(IntentKind::Corridor, swarm))
        {
            grid.corridor_cost_multiplier
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
    pub(crate) fn connected_point_with_budget(
        &self,
        start: Vec2,
        points: impl IntoIterator<Item = Option<Vec2>>,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec2>, ()> {
        poll_with_budget(
            self.connected_point(start, points.into_iter().map(std::future::ready)),
            allow_work,
        )
    }

    pub(super) async fn connected_point(
        &self,
        start: Vec2,
        points: impl IntoIterator<Item = impl Future<Output = Option<Vec2>>>,
    ) -> Option<Vec2> {
        let mut checked_regions = HashSet::new();
        let mut points = points.into_iter();
        loop {
            WorkUnit::new().await;
            let point = points.next()?;
            let Some(point) = point.await else {
                continue;
            };
            if !self.async_point_clear(point).await {
                continue;
            }
            if self.async_segment_clear(start, point).await {
                return Some(point);
            }
            let mut unchecked = false;
            for connector in self.async_connectors(point).await {
                if let Some(region) = self.async_region(connector).await {
                    unchecked |= checked_regions.insert(region);
                }
            }
            if unchecked
                && matches!(
                    self.async_connectivity(start, RouteGoal::Point(point))
                        .await,
                    RouteOutcome::Found(_)
                )
            {
                return Some(point);
            }
        }
    }

    pub(crate) fn point_clear_with_budget(
        &self,
        point: Vec2,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<bool, ()> {
        poll_with_budget(self.async_point_clear(point), allow_work)
    }

    pub(crate) fn segment_clear_with_budget(
        &self,
        from: Vec2,
        to: Vec2,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<bool, ()> {
        poll_with_budget(
            async {
                self.async_point_clear(from).await
                    && self.async_point_clear(to).await
                    && self.async_segment_clear(from, to).await
            },
            allow_work,
        )
    }

    /// Reuse movement routing without making existing Corridor paint favor its own path.
    /// Return no outcome when the caller's planning allowance is exhausted.
    pub(crate) fn route_without_paint(
        &self,
        origin: Vec2,
        goal: RouteGoal,
        mut allow_work: impl FnMut() -> bool,
    ) -> Option<RouteOutcome> {
        let paint = PaintCosts::default();
        let future = async {
            if self.async_point_clear(origin).await {
                return self
                    .async_solve(origin, goal, &paint, SwarmId::PLAYER, false, true)
                    .await;
            }
            for start in self.clear_origins_in_tile(origin).await {
                WorkUnit::new().await;
                if let RouteOutcome::Found(mut route) = self
                    .async_solve(start, goal, &paint, SwarmId::PLAYER, false, true)
                    .await
                {
                    route.waypoints.insert(0, start);
                    return RouteOutcome::Found(route);
                }
            }
            RouteOutcome::Unreachable
        };
        poll_with_budget(future, &mut allow_work).ok()
    }

    async fn clear_origins_in_tile(&self, origin: Vec2) -> Vec<Vec2> {
        if !origin.is_finite() || !origin.cmpge(self.min).all() || !origin.cmplt(self.max).all() {
            return Vec::new();
        }
        let tile = world_to_cell(origin);
        let min = Self::cell(tile.as_vec2() * ZONE_BLOCK_SIZE);
        let max = Self::cell((tile + IVec2::ONE).as_vec2() * ZONE_BLOCK_SIZE);
        let mut origins = Vec::new();
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                WorkUnit::new().await;
                let point = Self::center(IVec2::new(x, y));
                if world_to_cell(point) == tile && self.async_point_clear(point).await {
                    origins.push(point);
                }
            }
        }
        origins.sort_by(|a, b| {
            a.distance_squared(origin)
                .total_cmp(&b.distance_squared(origin))
        });
        origins
    }

    pub(super) fn solve(
        &self,
        start: Vec2,
        goal: RouteGoal,
        grid: &IntentGrid,
        swarm: SwarmId,
        hauler: bool,
    ) -> RouteOutcome {
        let paint = PaintCosts {
            corridor_cost_multiplier: self
                .scheduler
                .lock()
                .unwrap()
                .corridor_cost_multiplier
                .unwrap_or_else(|| {
                    crate::tuning::Tuning::default()
                        .logistics
                        .corridor_cost_multiplier
                }),
            corridor_swarms: grid
                .iter_active_cells()
                .flat_map(|(_, cell)| cell.owners(IntentKind::Corridor))
                .collect(),
            cells: grid
                .iter_active_cells()
                .map(|(p, c)| (p, c.clone()))
                .collect(),
        };
        let future = self.async_solve(start, goal, &paint, swarm, hauler, false);
        poll_with_budget(future, &mut || true).expect("unlimited route search")
    }
    async fn async_solve(
        &self,
        start: Vec2,
        goal: RouteGoal,
        paint: &PaintCosts,
        swarm: SwarmId,
        hauler: bool,
        meter_probes: bool,
    ) -> RouteOutcome {
        match goal {
            RouteGoal::Point(end) => self.async_route(start, end, paint, swarm, hauler).await,
            RouteGoal::Range { center, radius } => {
                self.async_route_within_range(start, center, radius, paint, swarm, hauler)
                    .await
            }
            RouteGoal::Interaction(region) => {
                self.async_route_to_interaction(start, region, paint, swarm, hauler, meter_probes)
                    .await
            }
        }
    }
}

impl Navigation {
    /// Adopt new clearing and geometry together, rebuilding the shared clearing layer once.
    pub(crate) fn refresh_with_clearing(
        &mut self,
        grid: &IntentGrid,
        obstacles: Vec<Obstacle>,
        clearing: Vec<Obstacle>,
    ) {
        let clearing_changed = self.clearing != clearing;
        if clearing_changed {
            self.clearing = clearing;
            self.revision = self.revision.wrapping_add(1);
            self.clearance = super::next_clearance();
        }
        let geometry_changed = self.refresh(grid, obstacles);
        if clearing_changed && !geometry_changed {
            self.rebuild_clearing_layer();
        }
    }

    pub fn refresh_clearing(&mut self, clearing: Vec<Obstacle>) {
        if self.clearing != clearing {
            self.clearing = clearing;
            self.revision = self.revision.wrapping_add(1);
            self.clearance = super::next_clearance();
            self.rebuild_clearing_layer();
        }
    }

    /// The shared clearing layer, when the start lies outside every clearing footprint.
    fn clearing_layer_for(&self, start: Vec2) -> Option<&Arc<Navigation>> {
        self.clearing_layer
            .as_ref()
            .filter(|_| self.clearing.iter().all(|shape| shape.admits_body(start)))
    }

    /// Shared geometry for a start, or `None` when it needs a private variant.
    fn start_layer(&self, start: Vec2) -> Option<&Navigation> {
        if self.clearing.is_empty() {
            return Some(self);
        }
        self.clearing_layer_for(start).map(|layer| &**layer)
    }

    /// Requests outside every clearing footprint share one layer. An occupant gets a
    /// private variant without its own footprint so it can leave.
    fn request_geometry(&self, start: Vec2) -> Arc<Navigation> {
        if self.clearing.is_empty() {
            return Arc::new(self.shared_snapshot());
        }
        if let Some(layer) = self.clearing_layer_for(start) {
            return layer.clone();
        }
        let obstacles = self
            .obstacles
            .iter()
            .copied()
            .chain(
                self.clearing
                    .iter()
                    .copied()
                    .filter(|shape| shape.admits_body(start)),
            )
            .collect();
        Arc::new(self.variant(obstacles))
    }
}

impl Navigation {
    /// Establish static access without refining or returning a movement path.
    /// Moving origins share the exact fine-cell component already built in their chunk.
    /// Paint and nanobot occupancy cannot change this geometric answer.
    pub fn query_connectivity(&self, start: Vec2, goal: RouteGoal) -> ConnectivityStatus {
        self.query_connectivity_with_origin(start, goal).0
    }

    /// Retain the original attachment point when sharing a cached connection's endpoint.
    pub(crate) fn query_connectivity_with_origin(
        &self,
        start: Vec2,
        goal: RouteGoal,
    ) -> (ConnectivityStatus, Vec2) {
        let cell = Self::cell(start);
        let region = self
            .start_layer(start)
            .filter(|layer| layer.segment_clear(start, Self::center(cell)))
            .and_then(|layer| layer.cached_region(cell));
        let key = ConnectivityKey {
            revision: self.revision,
            origin: region.map_or(ConnectivityOrigin::Cell(cell), ConnectivityOrigin::Region),
            goal: goal.into(),
        };
        let mut scheduler = self.scheduler.lock().unwrap();
        let tick = scheduler.tick;
        if region.is_some() && !scheduler.connectivity_cached.contains_key(&key) {
            let cold_key = ConnectivityKey {
                origin: ConnectivityOrigin::Cell(cell),
                ..key
            };
            if let Some(&(id, touched)) = scheduler.connectivity_cached.get(&cold_key)
                && scheduler.requests.get(&id).is_some_and(|r| {
                    r.revision == self.revision && self.movement_clear(start, r.start)
                })
            {
                scheduler.connectivity_cached.remove(&cold_key);
                scheduler.connectivity_cached.insert(key, (id, touched));
            }
        }
        if let Some(&(id, _)) = scheduler.connectivity_cached.get(&key) {
            let reusable = scheduler.requests.get(&id).is_some_and(|request| {
                request.revision == self.revision
                    && (region.is_some() || self.movement_clear(start, request.start))
            });
            if reusable {
                scheduler.connectivity_cached.get_mut(&key).unwrap().1 = tick;
                let request = &scheduler.requests[&id];
                let status = match &request.result {
                    RouteStatus::Pending => ConnectivityStatus::Pending,
                    RouteStatus::Found(route) => ConnectivityStatus::Connected {
                        endpoint: *route.waypoints.last().unwrap(),
                    },
                    RouteStatus::Unreachable => ConnectivityStatus::Unreachable,
                };
                return (status, request.start);
            }
            scheduler.connectivity_cached.remove(&key);
            scheduler.requests.remove(&id);
        }
        drop(scheduler);
        let id = self.request(start, goal, SwarmId::PLAYER, false, RoutePriority::Routine);
        let mut scheduler = self.scheduler.lock().unwrap();
        scheduler.requests.get_mut(&id).unwrap().purpose = RequestPurpose::Connectivity;
        scheduler.connectivity_cached.insert(key, (id, tick));
        (ConnectivityStatus::Pending, start)
    }
}

impl Navigation {
    pub(super) async fn async_point_clear(&self, p: Vec2) -> bool {
        if !p.is_finite() || !p.cmpge(self.min).all() || !p.cmplt(self.max).all() {
            return false;
        }
        for id in self.obstacle_index.clearance_candidates(p, p).await {
            WorkUnit::new().await;
            if !self.obstacles[id].admits_body(p) {
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
        for id in self.obstacle_index.clearance_candidates(a, b).await {
            WorkUnit::new().await;
            if !self.obstacles[id].segment_clear(a, b) {
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
                                && super::super::segment_clears_all(&check.completed, a, b))
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
        let request = scheduler.requests.get_mut(&id).unwrap();
        request.access = Some(check);
        request.purpose = RequestPurpose::Construction;
        scheduler.access_cached.insert(cache_key, (id, tick));
        AccessStatus::Pending
    }
    async fn async_connected(
        &self,
        a: crate::nanobot::InteractionRegion,
        b: crate::nanobot::InteractionRegion,
        _paint: &PaintCosts,
        _swarm: SwarmId,
    ) -> bool {
        for point in a.candidates(Vec2::ZERO) {
            WorkUnit::new().await;
            if matches!(
                self.async_connectivity(point, RouteGoal::Interaction(b))
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
        let baseline = self.variant(check.baseline.clone());
        let completed = self.variant(check.completed.clone());
        let mut builder_reaches = false;
        for &builder in &check.builders {
            WorkUnit::new().await;
            if matches!(
                completed
                    .async_connectivity(builder, RouteGoal::Interaction(check.target))
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

    /// A metered search: `units` work units, with an unmetered nested search
    /// after every third unit. Returns the units it ran.
    async fn metered_search(units: usize, log: &std::cell::RefCell<Vec<usize>>) -> usize {
        for unit in 0..units {
            log.borrow_mut().push(unit);
            if unit % 3 == 2 {
                let nested = async {
                    for _ in 0..4 {
                        WorkUnit::new().await;
                    }
                };
                let mut nested_units = 0;
                poll_with_budget(nested, &mut || {
                    nested_units += 1;
                    true
                })
                .unwrap();
                assert_eq!(nested_units, 5, "nested searches poll once per unit");
            }
            WorkUnit::new().await;
        }
        units
    }

    #[test]
    fn polling_several_units_at_once_matches_polling_one_unit_at_a_time() {
        for (units, quantum) in [(0, 1), (1, 1), (7, 1), (7, 3), (7, 8), (12, 4), (40, 128)] {
            let one_log = Default::default();
            let mut one = std::pin::pin!(metered_search(units, &one_log));
            let mut one_polls = Vec::new();
            loop {
                let mut used = 0;
                let mut ready = None;
                for _ in 0..quantum {
                    used += 1;
                    if let Poll::Ready(result) =
                        one.as_mut().poll(&mut Context::from_waker(Waker::noop()))
                    {
                        ready = Some(result);
                        break;
                    }
                }
                one_polls.push((used, one_log.borrow().len()));
                if ready.is_some() {
                    break;
                }
            }

            let many_log = Default::default();
            let mut many = std::pin::pin!(metered_search(units, &many_log));
            let mut many_polls = Vec::new();
            loop {
                let (poll, used) = poll_units(many.as_mut(), quantum);
                many_polls.push((used, many_log.borrow().len()));
                if poll.is_ready() {
                    break;
                }
            }

            assert_eq!(
                many_polls, one_polls,
                "{units} units in quanta of {quantum}"
            );
            assert_eq!(ALLOWANCE.get(), 0);
        }
    }

    #[test]
    fn clearance_identity_changes_exactly_when_clearance_inputs_change() {
        let grid = IntentGrid::new(8, 8);
        let rock = Obstacle::Circle {
            center: Vec2::new(100.0, 0.0),
            radius: 40.0,
        };
        let footprint = Obstacle::Rectangle {
            center: Vec2::new(-200.0, 0.0),
            half: Vec2::splat(30.0),
        };
        let mut navigation = Navigation::new(&grid, vec![rock]);
        let other = Navigation::new(&grid, vec![rock]);
        assert_ne!(
            navigation.clearance_identity(),
            other.clearance_identity(),
            "separate hierarchies never share an identity",
        );

        // Each step: whether it changes bounds, obstacles or clearing.
        type Change = fn(&mut Navigation, &IntentGrid, Obstacle, Obstacle);
        let steps: [(&str, bool, Change); 8] = [
            ("same obstacles", false, |navigation, grid, rock, _| {
                navigation.refresh(grid, vec![rock]);
            }),
            ("new obstacle", true, |navigation, grid, rock, footprint| {
                navigation.refresh(grid, vec![rock, footprint]);
            }),
            ("larger bounds", true, |navigation, _, rock, footprint| {
                navigation.refresh(&IntentGrid::new(10, 10), vec![rock, footprint]);
            }),
            ("same clearing", false, |navigation, _, _, _| {
                navigation.refresh_clearing(Vec::new());
            }),
            ("new clearing", true, |navigation, _, _, footprint| {
                navigation.refresh_clearing(vec![footprint]);
            }),
            (
                "same geometry and clearing",
                false,
                |navigation, _, rock, footprint| {
                    navigation.refresh_with_clearing(
                        &IntentGrid::new(10, 10),
                        vec![rock, footprint],
                        vec![footprint],
                    );
                },
            ),
            (
                "cleared clearing",
                true,
                |navigation, _, rock, footprint| {
                    navigation.refresh_with_clearing(
                        &IntentGrid::new(10, 10),
                        vec![rock, footprint],
                        Vec::new(),
                    );
                },
            ),
            ("fewer obstacles", true, |navigation, _, rock, _| {
                navigation.refresh_with_clearing(&IntentGrid::new(10, 10), vec![rock], Vec::new());
            }),
        ];
        for (step, changes, apply) in steps {
            let before = navigation.clearance_identity();
            apply(&mut navigation, &grid, rock, footprint);
            assert_eq!(navigation.clearance_identity() != before, changes, "{step}");
        }
    }

    #[test]
    fn connectivity_uses_exact_components_without_refining_routes() {
        let grid = IntentGrid::new(8, 8);
        let mut navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(36.0, 216.0),
            }],
        );
        let start = Vec2::new(-252.0, 36.0);
        let goal = RouteGoal::Point(Vec2::new(252.0, 36.0));
        let mut status = navigation.query_connectivity(start, goal);
        let mut fine = 0;
        for _ in 0..1000 {
            if status != ConnectivityStatus::Pending {
                break;
            }
            fine += navigation.advance(&grid, 2048).fine_expansions;
            status = navigation.query_connectivity(start, goal);
        }
        assert_eq!(
            status,
            ConnectivityStatus::Connected {
                endpoint: Vec2::new(252.0, 36.0)
            }
        );
        assert_eq!(fine, 0, "reachability must not refine a discarded path");
        assert_eq!(
            navigation.query_connectivity(Vec2::new(-324.0, 108.0), goal),
            status,
            "moving within the same exact component must reuse its access result"
        );
        navigation.refresh(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(36.0, 5000.0),
            }],
        );
        assert_eq!(
            navigation.query_connectivity(start, goal),
            ConnectivityStatus::Pending
        );
        for _ in 0..1000 {
            navigation.advance(&grid, 2048);
            status = navigation.query_connectivity(start, goal);
            if status != ConnectivityStatus::Pending {
                break;
            }
        }
        assert_eq!(
            status,
            ConnectivityStatus::Unreachable,
            "a full dividing wall separates exact components"
        );
    }

    #[test]
    fn stale_entries_expire_oldest_first_and_at_most_sixty_four_per_tick() {
        let grid = IntentGrid::new(4, 4);
        let mut navigation = Navigation::new(&grid, vec![]);
        let query = |index: usize| {
            navigation
                .query_connectivity(Vec2::ZERO, RouteGoal::Point(Vec2::new(index as f32, 100.0)))
        };
        let older = navigation.work().tick;
        for index in 0..100 {
            query(index);
        }
        navigation.advance(&grid, 100_000);
        for index in 100..200 {
            query(index);
        }
        navigation.advance(&grid, 100_000);
        assert!(matches!(query(150), ConnectivityStatus::Connected { .. }));
        navigation.refresh(&grid, vec![Obstacle::deposit(Vec2::splat(900.0), 10.0)]);

        let retained = |navigation: &Navigation| {
            let scheduler = navigation.scheduler.lock().unwrap();
            let older_entries = scheduler
                .connectivity_cached
                .values()
                .filter(|(_, touched)| *touched == older)
                .count();
            (
                older_entries,
                scheduler.connectivity_cached.len(),
                scheduler.requests.len(),
            )
        };
        let mut expected = 200;
        for _ in 0..4 {
            let work = navigation.advance(&grid, 100_000);
            assert_eq!(work.work, 0, "expiring answers must not be solved again");
            assert_eq!(work.pending, 0, "expiring answers are not pending work");
            expected -= expected.min(64);
            let (older_entries, entries, requests) = retained(&navigation);
            assert_eq!((entries, requests), (expected, expected));
            assert_eq!(older_entries, expected.saturating_sub(100));
        }
        assert_eq!(expected, 0);
    }

    #[test]
    fn hauler_without_owned_corridors_uses_clear_direct_route() {
        let mut grid = IntentGrid::new(8, 8);
        grid.paint(IVec2::ZERO, IntentKind::Corridor, SwarmId(7));
        let navigation = Navigation::new(&grid, vec![]);
        let id = navigation.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::new(100.0, 0.0)),
            SwarmId::PLAYER,
            true,
            RoutePriority::Routine,
        );
        let work = navigation.advance(&grid, 1);
        assert!(
            matches!(navigation.poll(id), RouteStatus::Found(_)),
            "hostile paint must not delay a clear hauler route"
        );
        assert_eq!(work.coarse_expansions + work.fine_expansions, 0);
    }

    #[test]
    fn bounded_quanta_serve_short_routes_behind_long_routes() {
        let grid = IntentGrid::new(40, 40);
        let navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(72.0, 720.0),
            }],
        );
        navigation.request(
            Vec2::new(-252.0, 36.0),
            RouteGoal::Point(Vec2::new(252.0, 36.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        let quick = navigation.request(
            Vec2::new(-500.0, -500.0),
            RouteGoal::Point(Vec2::new(-600.0, -500.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        navigation.advance(&grid, 256);
        assert!(matches!(navigation.poll(quick), RouteStatus::Found(_)));
    }

    #[test]
    fn cheap_movement_is_not_blocked_by_an_older_expensive_probe() {
        let grid = IntentGrid::new(40, 40);
        let navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(72.0, 720.0),
            }],
        );
        navigation.query_connectivity(
            Vec2::new(-252.0, 36.0),
            RouteGoal::Point(Vec2::new(252.0, 36.0)),
        );
        let movement = navigation.request(
            Vec2::new(-500.0, -500.0),
            RouteGoal::Point(Vec2::new(-600.0, -500.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        navigation.advance(&grid, 128);
        assert!(
            matches!(navigation.poll(movement), RouteStatus::Found(_)),
            "an unrelated background detour must not stall clear movement"
        );
    }

    #[test]
    fn background_work_progresses_while_movement_is_expensive() {
        let grid = IntentGrid::new(40, 40);
        let navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(72.0, 720.0),
            }],
        );
        navigation.request(
            Vec2::new(-252.0, 36.0),
            RouteGoal::Point(Vec2::new(252.0, 36.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Clearing,
        );
        let probe = || {
            navigation.query_connectivity(
                Vec2::new(-500.0, -500.0),
                RouteGoal::Point(Vec2::new(-600.0, -500.0)),
            )
        };
        assert_eq!(probe(), ConnectivityStatus::Pending);
        navigation.advance(&grid, 128);
        assert!(
            matches!(probe(), ConnectivityStatus::Connected { .. }),
            "sustained movement must leave capacity for background planning"
        );
    }

    #[test]
    fn distant_rocks_do_not_consume_local_direct_route_allowance() {
        let grid = IntentGrid::new(1000, 1000);
        let obstacles = (0..300)
            .map(|index| Obstacle::Rectangle {
                center: Vec2::new(20_000.0 + index as f32 * 720.0, 20_000.0),
                half: Vec2::splat(100.0),
            })
            .collect();
        let navigation = Navigation::new(&grid, obstacles);
        let request = navigation.request(
            Vec2::ZERO,
            RouteGoal::Point(Vec2::new(100.0, 0.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        navigation.advance(&grid, 1);
        let RouteStatus::Found(route) = navigation.poll(request) else {
            panic!("local clear segment must complete with one work unit despite distant terrain")
        };
        assert!((route.cost - 100.0).abs() < 0.001);
    }

    #[test]
    fn indexed_geometry_keeps_bucket_edges_huge_shapes_and_long_segments_solid() {
        let grid = IntentGrid::new(1000, 1000);
        let brute_point_clear = |navigation: &Navigation, point: Vec2| {
            point.is_finite()
                && point.cmpge(navigation.min).all()
                && point.cmplt(navigation.max).all()
                && navigation
                    .obstacles
                    .iter()
                    .all(|obstacle| obstacle.admits_body(point))
        };
        let brute_segment_clear = |navigation: &Navigation, a: Vec2, b: Vec2| {
            brute_point_clear(navigation, a)
                && brute_point_clear(navigation, b)
                && navigation
                    .obstacles
                    .iter()
                    .all(|obstacle| obstacle.segment_clear(a, b))
        };
        let assert_matches_brute_force =
            |navigation: &Navigation, points: &[Vec2], segments: &[(Vec2, Vec2)]| {
                for &point in points {
                    assert_eq!(
                        navigation.point_clear(point),
                        brute_point_clear(navigation, point)
                    );
                }
                for &(a, b) in segments {
                    let expected = brute_segment_clear(navigation, a, b);
                    assert_eq!(navigation.segment_clear(a, b), expected);
                    assert_eq!(navigation.segment_clear(b, a), expected);
                }
            };

        let bucket_edge_circle = Obstacle::Circle {
            center: Vec2::new(-580.0, -572.0),
            radius: 10.0,
        };
        let huge_global = Obstacle::Rectangle {
            center: Vec2::new(0.0, 2_000.0),
            half: Vec2::new(100_000.0, 36.0),
        };
        let mut navigation = Navigation::new(&grid, vec![bucket_edge_circle, huge_global]);
        let points = [
            Vec2::new(-614.0, -572.0),
            Vec2::new(-624.0, -572.0),
            Vec2::new(-580.0, -528.0),
            Vec2::new(-535.0, -572.0),
            Vec2::new(20_000.0, 2_050.0),
            Vec2::new(20_000.0, 2_071.0),
        ];
        let segments = [
            (Vec2::new(-700.0, -572.0), Vec2::new(-500.0, -572.0)),
            (Vec2::new(-700.0, -528.0), Vec2::new(-500.0, -528.0)),
            (Vec2::new(-50_000.0, -572.0), Vec2::new(50_000.0, -572.0)),
            (Vec2::new(-50_000.0, 2_070.0), Vec2::new(50_000.0, 2_070.0)),
            (Vec2::new(20_000.0, 1_900.0), Vec2::new(20_000.0, 2_100.0)),
        ];
        assert_matches_brute_force(&navigation, &points, &segments);

        navigation.refresh(
            &grid,
            vec![Obstacle::Circle {
                center: Vec2::new(580.0, 0.0),
                radius: 10.0,
            }],
        );
        let refreshed_points = [
            Vec2::new(550.0, 0.0),
            Vec2::new(535.0, 0.0),
            Vec2::new(-580.0, -572.0),
        ];
        let refreshed_segments = [
            (Vec2::new(500.0, 0.0), Vec2::new(650.0, 0.0)),
            (Vec2::new(500.0, 44.0), Vec2::new(650.0, 44.0)),
            (Vec2::new(-50_000.0, 0.0), Vec2::new(50_000.0, 0.0)),
        ];
        assert_matches_brute_force(&navigation, &refreshed_points, &refreshed_segments);

        navigation.refresh(&grid, vec![]);
        assert_matches_brute_force(&navigation, &points, &segments);
    }

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
        let query =
            || navigation.query_connectivity(Vec2::ZERO, RouteGoal::Point(Vec2::splat(100.0)));
        for _ in 0..100 {
            assert_eq!(query(), ConnectivityStatus::Pending);
        }
        let work = navigation.advance(&grid, 2);
        assert_eq!(work.completed, 1);
        assert_eq!(work.pending, 0);
        assert_eq!(
            query(),
            ConnectivityStatus::Connected {
                endpoint: Vec2::splat(100.0)
            }
        );
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
