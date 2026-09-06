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
    async fn async_connectivity(&self, start: Vec2, goal: RouteGoal) -> RouteOutcome {
        if !self.async_point_clear(start).await {
            return RouteOutcome::Unreachable;
        }
        let mut endpoints = match goal {
            RouteGoal::Point(end) => vec![end],
            RouteGoal::Interaction(region) => {
                if region.contains(start) {
                    return RouteOutcome::Found(Route {
                        waypoints: vec![start],
                        cost: 0.0,
                    });
                }
                region.candidates(start)
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
                points
            }
        };
        endpoints.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        let paint = PaintCosts::default();
        for endpoint in endpoints {
            WorkUnit::new().await;
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
    access_cached: HashMap<(u64, u64), (RouteRequestId, u64)>,
    connectivity_cached: HashMap<String, (RouteRequestId, u64)>,
}
#[derive(Default)]
struct PaintCosts {
    corridor_swarms: HashSet<SwarmId>,
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
        let stale_connectivity: Vec<_> = scheduler
            .connectivity_cached
            .iter()
            .filter(|(_, (id, touched))| {
                tick.saturating_sub(*touched) > 120
                    || scheduler
                        .requests
                        .get(id)
                        .is_none_or(|r| r.revision != self.revision)
            })
            .map(|(key, (id, _))| (key.clone(), *id))
            .collect();
        for (key, id) in stale_connectivity {
            scheduler.connectivity_cached.remove(&key);
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
        let mut lanes = [Vec::new(), Vec::new()];
        for (&id, request) in &scheduler.requests {
            if !matches!(request.result, RouteStatus::Pending) {
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
        for request in scheduler.requests.values() {
            if !matches!(request.result, RouteStatus::Pending) {
                continue;
            }
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
                let obstacles = if self.clearing.is_empty() {
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
                };
                let obstacle_index = if self.clearing.is_empty() {
                    self.obstacle_index.clone()
                } else {
                    Arc::new(ObstacleIndex::new(&obstacles))
                };
                let snapshot = Navigation {
                    min: self.min,
                    max: self.max,
                    obstacles,
                    obstacle_index,
                    clearing: Vec::new(),
                    revision: self.revision,
                    chunks: if self.clearing.is_empty() {
                        self.chunks.clone()
                    } else {
                        Arc::new(Mutex::new(HashMap::new()))
                    },
                    connections: if self.clearing.is_empty() {
                        self.connections.clone()
                    } else {
                        Default::default()
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
                let connectivity = request.purpose == RequestPurpose::Connectivity;
                request.future = Some(Box::pin(async move {
                    if let Some(check) = access {
                        return snapshot.async_access(&check, &paint).await;
                    }
                    if connectivity {
                        return snapshot.async_connectivity(start, goal).await;
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
            request.last_served = scheduler.tick;
            let mut context = Context::from_waker(Waker::noop());
            for _ in 0..QUANTUM.min(stop - work.work) {
                work.work += 1;
                if request.purpose == RequestPurpose::Movement {
                    work.movement_work += 1;
                } else {
                    work.background_work += 1;
                }
                if let Poll::Ready(result) =
                    request.future.as_mut().unwrap().as_mut().poll(&mut context)
                {
                    request.result = match result {
                        RouteOutcome::Found(route) => RouteStatus::Found(route),
                        RouteOutcome::Unreachable => RouteStatus::Unreachable,
                    };
                    request.future = None;
                    work.completed += 1;
                    work.max_latency_ticks = work
                        .max_latency_ticks
                        .max(scheduler.tick - request.submitted);
                    break;
                }
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
            corridor_swarms: grid
                .iter_active_cells()
                .flat_map(|(_, cell)| cell.owners(IntentKind::Corridor))
                .collect(),
            cells: grid
                .iter_active_cells()
                .map(|(p, c)| (p, c.clone()))
                .collect(),
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
    /// Establish static access without refining or returning a movement path.
    /// Moving origins share the exact fine-cell component already built in their chunk.
    /// Paint and nanobot occupancy cannot change this geometric answer.
    pub fn query_connectivity(&self, start: Vec2, goal: RouteGoal) -> ConnectivityStatus {
        let cell = Self::cell(start);
        let region = if self.clearing.is_empty() && self.segment_clear(start, Self::center(cell)) {
            self.chunks
                .lock()
                .unwrap()
                .get(&Self::chunk(cell))
                .and_then(|chunk| chunk.regions.get(&cell))
                .copied()
        } else {
            None
        };
        let key = format!(
            "{region:?}:{cell_key:?}:{goal:?}",
            cell_key = region.unwrap_or(cell)
        );
        let mut scheduler = self.scheduler.lock().unwrap();
        let tick = scheduler.tick;
        if region.is_some() && !scheduler.connectivity_cached.contains_key(&key) {
            let cold_key = format!("{none:?}:{cell:?}:{goal:?}", none = None::<IVec2>);
            if let Some(&(id, touched)) = scheduler.connectivity_cached.get(&cold_key)
                && scheduler.requests.get(&id).is_some_and(|r| {
                    r.revision == self.revision && self.movement_clear(start, r.start)
                })
            {
                scheduler.connectivity_cached.remove(&cold_key);
                scheduler
                    .connectivity_cached
                    .insert(key.clone(), (id, touched));
            }
        }
        if let Some(&(id, _)) = scheduler.connectivity_cached.get(&key) {
            let reusable = scheduler.requests.get(&id).is_some_and(|request| {
                request.revision == self.revision
                    && (region.is_some() || self.movement_clear(start, request.start))
            });
            if reusable {
                scheduler.connectivity_cached.get_mut(&key).unwrap().1 = tick;
                return match &scheduler.requests[&id].result {
                    RouteStatus::Pending => ConnectivityStatus::Pending,
                    RouteStatus::Found(route) => ConnectivityStatus::Connected {
                        endpoint: *route.waypoints.last().unwrap(),
                    },
                    RouteStatus::Unreachable => ConnectivityStatus::Unreachable,
                };
            }
            scheduler.connectivity_cached.remove(&key);
            scheduler.requests.remove(&id);
        }
        drop(scheduler);
        let id = self.request(start, goal, SwarmId::PLAYER, false, RoutePriority::Routine);
        let mut scheduler = self.scheduler.lock().unwrap();
        scheduler.requests.get_mut(&id).unwrap().purpose = RequestPurpose::Connectivity;
        scheduler.connectivity_cached.insert(key, (id, tick));
        ConnectivityStatus::Pending
    }
}

impl Navigation {
    async fn async_point_clear(&self, p: Vec2) -> bool {
        if !p.is_finite() || !p.cmpge(self.min).all() || !p.cmplt(self.max).all() {
            return false;
        }
        for id in self.obstacle_index.candidates(p, p) {
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
        for id in self.obstacle_index.candidates(a, b) {
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
        let request = scheduler.requests.get_mut(&id).unwrap();
        request.access = Some(check);
        request.purpose = RequestPurpose::Construction;
        scheduler.access_cached.insert(cache_key, (id, tick));
        AccessStatus::Pending
    }
    fn hypothetical(&self, obstacles: Vec<Obstacle>) -> Navigation {
        Navigation {
            min: self.min,
            max: self.max,
            obstacle_index: Arc::new(ObstacleIndex::new(&obstacles)),
            obstacles: Arc::new(obstacles),
            clearing: Vec::new(),
            revision: self.revision,
            chunks: Arc::new(Mutex::new(HashMap::new())),
            connections: Default::default(),
            scheduler: Mutex::new(Scheduler::default()),
            expansions: self.expansions.clone(),
            chunk_builds: Default::default(),
        }
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
        let baseline = self.hypothetical(check.baseline.clone());
        let completed = self.hypothetical(check.completed.clone());
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
        let mut navigation = Navigation::new(
            &grid,
            vec![Obstacle::Circle {
                center: Vec2::new(580.0, 0.0),
                radius: 10.0,
            }],
        );
        assert!(
            !navigation.point_clear(Vec2::new(550.0, 0.0)),
            "body radius crosses the 576-unit bucket boundary"
        );
        assert!(navigation.point_clear(Vec2::new(535.0, 0.0)));
        assert!(
            !navigation.segment_clear(Vec2::new(-50_000.0, 0.0), Vec2::new(50_000.0, 0.0)),
            "long segment fallback must retain small obstacles"
        );
        navigation.refresh(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(100_000.0, 36.0),
            }],
        );
        assert!(
            !navigation.point_clear(Vec2::new(20_000.0, 50.0)),
            "huge rectangles remain global candidates"
        );
        assert!(!navigation.segment_clear(Vec2::new(20_000.0, -100.0), Vec2::new(20_000.0, 100.0)));
        navigation.refresh(&grid, vec![]);
        assert!(
            navigation.point_clear(Vec2::new(20_000.0, 50.0)),
            "refresh must discard stale blockers"
        );
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
