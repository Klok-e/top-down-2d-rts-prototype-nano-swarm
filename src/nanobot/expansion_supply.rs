//! Rolling Worker replenishment and uncommitted Hauler time for facility expansion.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::{ecs::system::SystemParam, prelude::*};

use crate::{
    intent::IntentGrid,
    navigation::{
        BODY_RADIUS, CELL_WIDTH, ConnectivityStatus, Navigation, Obstacle, RouteGoal,
        RouteRequestId, RouteStatus, ShapeBits,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
    tuning::Tuning,
};

use super::{
    Cargo, Charger, HaulerAssignment, HaulerLoading, Health, InteractionRegion, Nanobot,
    NanobotType, OwnerSwarm, STOP_THRESHOLD, SupportCondition, Swarm, SwarmId, SwarmMember,
    interaction::CandidateParts, spending_capacity,
};

const WINDOW_SECONDS: u64 = 20;
const WINDOW_TICKS: u64 = WINDOW_SECONDS * crate::SIMULATION_HZ as u64;

#[derive(Default, Resource)]
pub struct ExpansionHistory {
    tick: u64,
    observed_since: HashMap<Entity, u64>,
    drops: VecDeque<(u64, SwarmId, Entity, u32)>,
    replenishment: HashMap<(SwarmId, Entity), u64>,
    haulers: HashMap<(SwarmId, Entity), HaulerTime>,
}

#[derive(Default)]
struct HaulerTime {
    ticks: VecDeque<bool>,
    idle: u32,
    last_seen: u64,
}

impl HaulerTime {
    fn observe(&mut self, tick: u64, idle: bool) {
        self.ticks.push_back(idle);
        self.idle += u32::from(idle);
        if self.ticks.len() > WINDOW_TICKS as usize {
            self.idle -= u32::from(self.ticks.pop_front().expect("window is full"));
        }
        self.last_seen = tick;
    }
}

impl ExpansionHistory {
    pub(crate) fn record_dropoff(&mut self, swarm: SwarmId, source: Entity, amount: u32) {
        // Gather delivery runs before observation in the same fixed tick.
        self.drops.push_back((self.tick + 1, swarm, source, amount));
        *self.replenishment.entry((swarm, source)).or_default() += u64::from(amount);
    }

    fn window_complete(&self, owner: Entity) -> bool {
        self.observed_since
            .get(&owner)
            .is_some_and(|since| self.tick - since >= WINDOW_TICKS)
    }
}

type Crew<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        &'static SwarmMember,
        &'static NanobotType,
        Option<&'static Health>,
        Has<HaulerAssignment>,
        Has<HaulerLoading>,
        Has<Cargo>,
    ),
    With<Nanobot>,
>;

pub(crate) fn observe_expansion_supply(
    mut history: ResMut<ExpansionHistory>,
    swarms: Query<Entity, With<Swarm>>,
    crew: Crew,
) {
    history.tick += 1;
    let tick = history.tick;
    while let Some(&(then, swarm, source, amount)) = history.drops.front() {
        if tick - then < WINDOW_TICKS {
            break;
        }
        history.drops.pop_front();
        let key = (swarm, source);
        let remaining = history.replenishment[&key] - u64::from(amount);
        if remaining == 0 {
            history.replenishment.remove(&key);
        } else {
            history.replenishment.insert(key, remaining);
        }
    }
    history
        .observed_since
        .retain(|owner, _| swarms.contains(*owner));
    for owner in &swarms {
        history.observed_since.entry(owner).or_insert(tick - 1);
    }
    for (entity, _, member, kind, health, assigned, loading, carrying) in &crew {
        if *kind == NanobotType::Hauler && health.is_none_or(|health| health.current > 0) {
            history
                .haulers
                .entry((member.0, entity))
                .or_default()
                .observe(tick, !(assigned || loading || carrying));
        }
    }
    history.haulers.retain(|_, time| time.last_seen == tick);
}

#[derive(SystemParam)]
#[allow(clippy::type_complexity)]
pub struct ExpansionSupply<'w, 's> {
    history: Option<Res<'w, ExpansionHistory>>,
    navigation: Res<'w, Navigation>,
    paths: Local<'s, DeliveryPaths>,
    crew: Crew<'w, 's>,
    stockpiles: Query<
        'w,
        's,
        (
            Entity,
            &'static Stockpile,
            &'static Transform,
            &'static OwnerSwarm,
            Option<&'static StockpileRole>,
            Option<&'static SupportCondition>,
        ),
    >,
    chargers: Query<
        'w,
        's,
        (
            &'static Charger,
            &'static Transform,
            &'static OwnerSwarm,
            Option<&'static SupportCondition>,
        ),
    >,
}

struct Source {
    center: Vec2,
    half: Vec2,
    region: InteractionRegion,
    minerals: f64,
    load: f64,
    transfer_ticks_per_mineral: f64,
    spending: Vec<usize>,
    pending_spending: Vec<usize>,
}

pub(super) struct SupplyBudget {
    sources: Vec<Source>,
    spending: Vec<f64>,
    haulers: Vec<(Vec2, u32)>,
    expansion: f64,
    speed: f64,
}

impl ExpansionSupply<'_, '_> {
    pub(super) fn begin_search(&mut self) {
        self.paths.begin(&self.navigation);
    }

    pub(super) fn budget(
        &self,
        owner: Entity,
        swarm: SwarmId,
        facilities: &[Transform],
        grid: &IntentGrid,
        tuning: &Tuning,
    ) -> Option<SupplyBudget> {
        let history = self.history.as_ref()?;
        if !history.window_complete(owner) {
            return None;
        }

        let rate = f64::from(tuning.production.cost_per_bot)
            / f64::from(tuning.production.ticks_per_bot.max(1));
        let expansion = rate * WINDOW_TICKS as f64;
        let mut defenders = 0;
        let mut haulers = Vec::new();
        for (entity, transform, member, kind, health, ..) in &self.crew {
            if member.0 != swarm || health.is_some_and(|health| health.current == 0) {
                continue;
            }
            match *kind {
                NanobotType::Defender => defenders += 1,
                NanobotType::Hauler => {
                    if let Some(time) = history
                        .haulers
                        .get(&(swarm, entity))
                        .filter(|time| time.idle > 0)
                    {
                        haulers.push((transform.translation.truncate(), time.idle));
                    }
                }
                NanobotType::Worker => {}
            }
        }
        if haulers.is_empty() {
            return None;
        }

        let charge = f64::from(spending_capacity(0, defenders, tuning)) * WINDOW_SECONDS as f64;
        let logistics = &tuning.logistics;
        let carry = logistics.hauler_carry_capacity.max(1);
        let mut sources = self
            .stockpiles
            .iter()
            .filter_map(
                |(entity, stockpile, transform, stockpile_owner, role, condition)| {
                    let minerals = *history.replenishment.get(&(swarm, entity))? as f64;
                    let load = carry.min(stockpile.capacity);
                    if load == 0 {
                        return None;
                    }
                    (stockpile_owner.0 == owner
                        && stockpile.kind == ResourceKind::Minerals
                        && role != Some(&StockpileRole::Sink)
                        && condition.is_none_or(SupportCondition::is_operational))
                    .then_some((
                        entity.to_bits(),
                        Source {
                            center: transform.translation.truncate(),
                            half: interaction_bounds(transform),
                            region: InteractionRegion::structure(transform),
                            minerals,
                            load: f64::from(load),
                            transfer_ticks_per_mineral: transfer_ticks(load, tuning)
                                / f64::from(load),
                            spending: Vec::new(),
                            pending_spending: Vec::new(),
                        },
                    ))
                },
            )
            .collect::<Vec<_>>();
        let total_spending = expansion * facilities.len() as f64 + charge;
        if !covers(
            sources.iter().map(|(_, source)| source.minerals).sum(),
            total_spending + expansion,
        ) {
            return None;
        }
        let fastest_transfer = sources
            .iter()
            .map(|(_, source)| source.transfer_ticks_per_mineral)
            .min_by(f64::total_cmp)?;
        if !covers(
            haulers.iter().map(|(_, idle)| f64::from(*idle)).sum(),
            expansion * fastest_transfer,
        ) {
            return None;
        }

        let mut spending = vec![expansion; facilities.len()];
        if charge > 0.0 {
            spending.push(charge);
        }
        let chargers = self
            .chargers
            .iter()
            .filter_map(|(charger, transform, charger_owner, condition)| {
                (charger_owner.0 == owner
                    && condition.is_none_or(SupportCondition::is_operational)
                    && grid.cell(charger.cell).is_some_and(|cell| {
                        cell.has_owned(crate::intent::IntentKind::Defend, swarm)
                    }))
                .then_some(transform)
            })
            .collect::<Vec<_>>();

        sources.sort_unstable_by_key(|(entity, _)| *entity);
        let mut sources = sources
            .into_iter()
            .map(|(_, source)| source)
            .collect::<Vec<_>>();
        for source in &mut sources {
            for (index, terminal) in facilities.iter().enumerate() {
                match can_supply(&self.navigation, source, terminal) {
                    Some(true) => source.spending.push(index),
                    Some(false) => {}
                    None => source.pending_spending.push(index),
                }
            }
            if charge == 0.0 {
                continue;
            }
            // Until a Charger is available, reserve upkeep where the current economy can use it.
            let mut connected = !source.spending.is_empty() && chargers.is_empty();
            let mut pending = !source.pending_spending.is_empty() && chargers.is_empty();
            for terminal in &chargers {
                match can_supply(&self.navigation, source, terminal) {
                    Some(true) => {
                        connected = true;
                        break;
                    }
                    Some(false) => {}
                    None => pending = true,
                }
            }
            if connected {
                source.spending.push(facilities.len());
            } else if pending {
                source.pending_spending.push(facilities.len());
            }
        }
        // Skip the placement search if even the most favorable pending links cannot cover current spending.
        if spending.iter().enumerate().any(|(target, &need)| {
            !covers(
                sources
                    .iter()
                    .filter(|source| {
                        source.spending.contains(&target)
                            || source.pending_spending.contains(&target)
                    })
                    .map(|source| source.minerals)
                    .sum(),
                need,
            )
        }) {
            return None;
        }

        Some(SupplyBudget {
            sources,
            spending,
            haulers,
            expansion,
            speed: f64::from(tuning.logistics.bot_speed),
        })
    }

    /// An unresolved earlier site can still pass and must keep its place in the search.
    pub(super) fn supplies_site(
        &mut self,
        budget: &SupplyBudget,
        site: &Transform,
    ) -> Option<bool> {
        let destination = InteractionRegion::structure(site);
        let footprint = Obstacle::structure(site);
        let (center, _) = footprint.bounds();
        let half = interaction_bounds(site);
        let mut confirmed = budget.rates();
        let mut possible = budget.rates();
        for (index, source) in budget.sources.iter().enumerate() {
            let dropoff = destination.approach(source.center);
            let pickup = source.region.approach(dropoff);
            if delivery_clear(&self.navigation, footprint, dropoff, pickup) {
                let ticks = budget.trip_cost(source, f64::from(dropoff.distance(pickup)));
                for (hauler, &(position, _)) in budget.haulers.iter().enumerate() {
                    if delivery_clear(&self.navigation, footprint, position, pickup) {
                        confirmed[hauler][index] = ticks;
                    }
                }
            }

            // Straight-line separation of the work areas bounds any route between them.
            let gap = (center - source.center).abs() - source.half - half;
            let ticks = budget.trip_cost(source, f64::from(gap.max(Vec2::ZERO).length()));
            for rates in &mut possible {
                rates[index] = ticks;
            }
        }

        if budget.covers_site(&confirmed, false) {
            return Some(true);
        }
        if !budget.covers_site(&possible, true) {
            return Some(false);
        }

        possible.clone_from(&confirmed);
        let dropoff_parts = destination.approach_parts();
        let mut routes = SiteRoutes {
            navigation: &self.navigation,
            paths: &mut self.paths,
            footprint,
            destination,
            dropoff_parts: &dropoff_parts,
            budget,
            confirmed,
            possible,
            probe: None,
            pending_connectivity: false,
        };
        for (index, source) in budget.sources.iter().enumerate() {
            let dropoffs = destination.candidates_from(&dropoff_parts, source.center);
            let mut pickups = source.region.candidates(center);
            for &dropoff in &dropoffs {
                let pickup = source.region.approach(dropoff);
                if pickups
                    .iter()
                    .all(|old| old.distance_squared(pickup) > 0.01)
                {
                    pickups.push(pickup);
                }
            }
            for pickup in pickups {
                routes.add_pickup(index, source, pickup, &dropoffs);
            }
        }
        routes.result()
    }
}

fn transfer_ticks(load: u32, tuning: &Tuning) -> f64 {
    let logistics = &tuning.logistics;
    let production = &tuning.production;
    let transfer = logistics.hauler_transfer_per_pulse.max(1);
    let hopper = production.input_capacity.max(1);
    let work = f64::from(logistics.material_work_ticks);
    let cycle = f64::from(production.ticks_per_bot.max(1));
    let loading = f64::from(load.div_ceil(transfer)) * work;
    let unloading = f64::from(load.div_ceil(transfer.min(hopper))) * work;
    let starts = load
        .saturating_sub(hopper)
        .div_ceil(production.cost_per_bot.max(1));
    if starts == 0 {
        return loading + unloading;
    }
    // If the hopper starts a cycle before the first delivery pulse, those queued
    // minerals take up at least a cycle's worth of space and require one extra start.
    let space_wait = f64::from(starts - 1) * cycle + work.min(cycle);
    loading + unloading.max(space_wait)
}

fn interaction_bounds(transform: &Transform) -> Vec2 {
    let (_, half) = Obstacle::structure(transform).bounds();
    let x = (transform.rotation * Vec3::X).truncate().abs();
    let y = (transform.rotation * Vec3::Y).truncate().abs();
    x * half.x + y * half.y + Vec2::splat(BODY_RADIUS + 2.0 * STOP_THRESHOLD + 0.001)
}

fn delivery_clear(navigation: &Navigation, footprint: Obstacle, start: Vec2, end: Vec2) -> bool {
    footprint.segment_clear(start, end)
        && navigation.point_clear_for_entry(start)
        && navigation.movement_clear(start, end)
}

impl SupplyBudget {
    fn rates(&self) -> Vec<Vec<f64>> {
        vec![vec![f64::INFINITY; self.sources.len()]; self.haulers.len()]
    }

    fn trip_cost(&self, source: &Source, distance: f64) -> f64 {
        2.0 * distance / self.speed / source.load + source.transfer_ticks_per_mineral
    }

    fn covers_site(&self, rates: &[Vec<f64>], include_pending: bool) -> bool {
        let reachable_supply = self
            .sources
            .iter()
            .enumerate()
            .filter(|(index, _)| rates.iter().any(|hauler| hauler[*index].is_finite()))
            .map(|(_, source)| source.minerals)
            .sum();
        if !covers(reachable_supply, self.expansion) {
            return false;
        }

        // Every Hauler needs only its cheapest accessible route to each Source.
        // Haulers with the same options can share one idle-time constraint.
        let mut pools = BTreeMap::<Vec<(usize, u64)>, u64>::new();
        let mut idle_capacity = 0.0;
        for (&(_, time), rates) in self.haulers.iter().zip(rates) {
            let fastest = rates.iter().copied().fold(f64::INFINITY, f64::min);
            idle_capacity += f64::from(time) / fastest;
            let reachable: Vec<_> = rates
                .iter()
                .enumerate()
                .filter_map(|(source, &ticks)| {
                    ticks.is_finite().then_some((source, ticks.to_bits()))
                })
                .collect();
            if !reachable.is_empty() {
                *pools.entry(reachable).or_default() += u64::from(time);
            }
        }
        if !covers(idle_capacity, self.expansion) {
            return false;
        }

        let limits = self
            .sources
            .iter()
            .map(|source| source.minerals)
            .chain(self.spending.iter().copied())
            .chain([self.expansion])
            .chain(pools.values().map(|&time| time as f64));
        let mut flow = Consumption::new(limits.collect());
        let target_start = self.sources.len();
        let expansion_row = target_start + self.spending.len();
        for (index, source) in self.sources.iter().enumerate() {
            for &target in &source.spending {
                flow.allow(&[(index, 1.0), (target_start + target, 1.0)]);
            }
            if include_pending {
                for &target in &source.pending_spending {
                    flow.allow(&[(index, 1.0), (target_start + target, 1.0)]);
                }
            }
        }
        for (index, reachable) in pools.into_keys().enumerate() {
            for (source, ticks) in reachable {
                flow.allow(&[
                    (source, 1.0),
                    (expansion_row, 1.0),
                    (expansion_row + 1 + index, f64::from_bits(ticks)),
                ]);
            }
        }

        // All spending, including one new facility, has a cap. The total can reach the
        // sum of those caps only if every terminal gets its full rate simultaneously.
        covers(
            flow.maximum(),
            self.spending.iter().sum::<f64>() + self.expansion,
        )
    }
}

fn connects(navigation: &Navigation, start: Vec2, goal: RouteGoal, approach: Vec2) -> Option<bool> {
    if navigation.movement_clear(start, approach) {
        return Some(true);
    }
    match navigation.query_connectivity(start, goal) {
        ConnectivityStatus::Connected { .. } => Some(true),
        ConnectivityStatus::Unreachable => Some(false),
        ConnectivityStatus::Pending => None,
    }
}

fn can_supply(navigation: &Navigation, source: &Source, terminal: &Transform) -> Option<bool> {
    let target = InteractionRegion::structure(terminal);
    let center = terminal.translation.truncate();
    let best = source.region.approach(center);
    if navigation.point_clear_for_entry(best)
        && navigation.movement_clear(best, target.approach(best))
    {
        return Some(true);
    }
    let mut pending = false;
    for pickup in std::iter::once(best).chain(source.region.candidates(center)) {
        if !navigation.point_clear_for_entry(pickup) {
            continue;
        }
        match connects(
            navigation,
            pickup,
            RouteGoal::Interaction(target),
            target.approach(pickup),
        ) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => pending = true,
        }
    }
    (!pending).then_some(false)
}

struct SiteRoutes<'a> {
    navigation: &'a Navigation,
    paths: &'a mut DeliveryPaths,
    footprint: Obstacle,
    destination: InteractionRegion,
    dropoff_parts: &'a CandidateParts,
    budget: &'a SupplyBudget,
    confirmed: Vec<Vec<f64>>,
    possible: Vec<Vec<f64>>,
    probe: Option<PathProbe>,
    pending_connectivity: bool,
}

impl SiteRoutes<'_> {
    fn add_pickup(&mut self, index: usize, source: &Source, pickup: Vec2, dropoffs: &[Vec2]) {
        if !self.footprint.admits_body(pickup) || !self.navigation.point_clear_for_entry(pickup) {
            return;
        }

        let candidates = self.destination.candidates_from(self.dropoff_parts, pickup);
        let nearest = candidates
            .first()
            .map_or(f64::INFINITY, |point| f64::from(pickup.distance(*point)));
        let mut direct = if self.destination.contains(pickup) {
            0.0
        } else {
            f64::INFINITY
        };
        for &point in candidates.iter().chain(dropoffs) {
            let distance = f64::from(pickup.distance(point));
            if distance < direct && delivery_clear(self.navigation, self.footprint, pickup, point) {
                direct = distance;
            }
        }
        let delivery = if nearest < direct {
            self.paths.estimate(
                self.navigation,
                self.footprint,
                pickup,
                PathProbe {
                    start: pickup,
                    goal: RouteGoal::Interaction(self.destination),
                },
            )
        } else {
            RouteEstimate::Known(direct)
        };
        let (distance, lower_bound) = match delivery {
            RouteEstimate::Known(distance) => (distance.min(direct), distance.min(direct)),
            RouteEstimate::Missing => (direct, direct),
            RouteEstimate::Connecting | RouteEstimate::Pending(_) => (direct, nearest),
        };
        if !lower_bound.is_finite() {
            return;
        }
        let cost = self.budget.trip_cost(source, distance);
        let possible_cost = self.budget.trip_cost(source, lower_bound);

        for (hauler, &(position, _)) in self.budget.haulers.iter().enumerate() {
            let possible = &mut self.possible[hauler][index];
            let confirmed = &mut self.confirmed[hauler][index];
            if *possible <= possible_cost && *confirmed <= cost {
                continue;
            }
            let reach =
                self.paths
                    .estimate_pickup(self.navigation, self.footprint, position, pickup);
            if matches!(reach, RouteEstimate::Missing) {
                continue;
            }
            if matches!(reach, RouteEstimate::Known(_)) {
                *confirmed = confirmed.min(cost);
            }
            if possible_cost >= *possible {
                continue;
            }
            *possible = possible_cost;
            if possible_cost < *confirmed {
                for estimate in [delivery, reach] {
                    match estimate {
                        RouteEstimate::Pending(probe) => {
                            self.probe.get_or_insert(probe);
                        }
                        RouteEstimate::Connecting => self.pending_connectivity = true,
                        _ => {}
                    }
                }
            }
        }
    }

    fn result(self) -> Option<bool> {
        if self.budget.covers_site(&self.confirmed, false) {
            return Some(true);
        }
        let pending = self.pending_connectivity
            || self.probe.is_some()
            || self
                .budget
                .sources
                .iter()
                .any(|source| !source.pending_spending.is_empty());
        if !pending || !self.budget.covers_site(&self.possible, true) {
            return Some(false);
        }
        if let Some(probe) = self.probe {
            self.paths.request(self.navigation, self.footprint, probe);
        }
        None
    }
}

type PathKey = (ShapeBits, DeliveryGoal);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DeliveryGoal {
    Pickup([u32; 2]),
    Dropoff([u32; 2], super::interaction::InteractionBits),
}

#[derive(Clone, Copy)]
struct PathProbe {
    start: Vec2,
    goal: RouteGoal,
}

impl PathProbe {
    fn key(self, footprint: Obstacle) -> PathKey {
        let goal = match self.goal {
            RouteGoal::Point(point) => DeliveryGoal::Pickup(point.to_array().map(f32::to_bits)),
            RouteGoal::Interaction(region) => {
                DeliveryGoal::Dropoff(self.start.to_array().map(f32::to_bits), region.bits())
            }
            RouteGoal::Range { .. } => unreachable!(),
        };
        (footprint.bits(), goal)
    }
}

#[derive(Clone, Copy)]
enum RouteEstimate {
    Known(f64),
    Connecting,
    Pending(PathProbe),
    Missing,
}

#[derive(Default)]
struct DeliveryPaths {
    geometry: Option<(u64, u64)>,
    pending: Option<(PathKey, PathProbe, RouteRequestId)>,
    finished: HashMap<PathKey, Vec<(Vec2, Option<f64>)>>,
}

impl DeliveryPaths {
    fn begin(&mut self, navigation: &Navigation) {
        let geometry = (navigation.clearance_identity(), navigation.revision());
        if self.geometry != Some(geometry) {
            if let Some((_, _, id)) = self.pending.take() {
                navigation.cancel(id);
            }
            self.finished.clear();
            self.geometry = Some(geometry);
            return;
        }
        let Some((key, probe, id)) = self.pending else {
            return;
        };
        let result = match navigation.poll(id) {
            RouteStatus::Pending => return,
            RouteStatus::Unreachable => None,
            RouteStatus::Found(route) => Some(match key.1 {
                DeliveryGoal::Pickup(_) => 0.0,
                DeliveryGoal::Dropoff(..) => f64::from(route.cost),
            }),
        };
        navigation.cancel(id);
        self.finished
            .entry(key)
            .or_default()
            .push((probe.start, result));
        self.pending = None;
    }

    fn estimate_pickup(
        &self,
        navigation: &Navigation,
        footprint: Obstacle,
        position: Vec2,
        pickup: Vec2,
    ) -> RouteEstimate {
        if delivery_clear(navigation, footprint, position, pickup) {
            return RouteEstimate::Known(0.0);
        }
        if !footprint.admits_body(position) || !navigation.point_clear_for_entry(position) {
            return RouteEstimate::Missing;
        }
        let cell_center = ((position / CELL_WIDTH).floor() + Vec2::splat(0.5)) * CELL_WIDTH;
        let start = if delivery_clear(navigation, footprint, position, cell_center) {
            cell_center
        } else {
            position
        };
        self.estimate(
            navigation,
            footprint,
            position,
            PathProbe {
                start,
                goal: RouteGoal::Point(pickup),
            },
        )
    }

    fn estimate(
        &self,
        navigation: &Navigation,
        footprint: Obstacle,
        origin: Vec2,
        probe: PathProbe,
    ) -> RouteEstimate {
        let key = probe.key(footprint);
        let can_attach = |point| {
            probe.start == point
                || (matches!(key.1, DeliveryGoal::Pickup(_))
                    && delivery_clear(navigation, footprint, origin, point))
        };
        if let Some((_, result)) = self
            .finished
            .get(&key)
            .into_iter()
            .flatten()
            .find(|(start, _)| can_attach(*start))
        {
            return result.map_or(RouteEstimate::Missing, RouteEstimate::Known);
        }
        if let Some((active_key, active, _)) = self.pending
            && key == active_key
            && can_attach(active.start)
        {
            return RouteEstimate::Pending(active);
        }
        // Disconnected paths in current geometry cannot be opened by the added footprint.
        if matches!(probe.goal, RouteGoal::Point(point) if navigation.movement_clear(probe.start, point))
        {
            return RouteEstimate::Pending(probe);
        }
        match navigation.query_connectivity(probe.start, probe.goal) {
            ConnectivityStatus::Connected { .. } => RouteEstimate::Pending(probe),
            ConnectivityStatus::Unreachable => RouteEstimate::Missing,
            ConnectivityStatus::Pending => RouteEstimate::Connecting,
        }
    }

    fn request(&mut self, navigation: &Navigation, footprint: Obstacle, probe: PathProbe) {
        // One forecast at a time shares the same navigation budget as normal movement.
        if self.pending.is_none() {
            let id = navigation.request_delivery_route(probe.start, probe.goal, footprint);
            self.pending = Some((probe.key(footprint), probe, id));
        }
    }
}

fn covers(available: f64, required: f64) -> bool {
    available + 1e-8 * required.max(1.0) >= required
}

/// Fractional mineral flows. Slack variables give a feasible empty allocation,
/// so simplex can maximize covered demand without a separate feasibility pass.
struct Consumption {
    limits: Vec<f64>,
    flows: Vec<Vec<(usize, f64)>>,
}

impl Consumption {
    fn new(limits: Vec<f64>) -> Self {
        Self {
            limits,
            flows: Vec::new(),
        }
    }

    fn allow(&mut self, constraints: &[(usize, f64)]) {
        self.flows.push(constraints.to_vec());
    }

    fn maximum(self) -> f64 {
        const EPSILON: f64 = 1e-10;
        let rows = self.limits.len();
        let flows = self.flows.len();
        let rhs = rows + flows;
        let mut table = vec![vec![0.0; rhs + 1]; rows + 1];
        let mut basic = (flows..rhs).collect::<Vec<_>>();
        for (row, limit) in self.limits.into_iter().enumerate() {
            table[row][rhs] = limit;
            table[row][flows + row] = 1.0;
        }
        for (column, constraints) in self.flows.into_iter().enumerate() {
            table[rows][column] = -1.0;
            for (row, coefficient) in constraints {
                table[row][column] = coefficient;
            }
        }

        // Bland's rule gives deterministic pivots and terminates with coincident caps.
        while let Some(column) = (0..rhs).find(|&column| table[rows][column] < -EPSILON) {
            let leaving = (0..rows)
                .filter(|&row| table[row][column] > EPSILON)
                .min_by(|&a, &b| {
                    let ratio = |row: usize| table[row][rhs] / table[row][column];
                    ratio(a)
                        .total_cmp(&ratio(b))
                        .then_with(|| basic[a].cmp(&basic[b]))
                });
            let Some(leaving) = leaving else {
                return 0.0;
            };
            basic[leaving] = column;
            let pivot = table[leaving][column];
            let (above, remainder) = table.split_at_mut(leaving);
            let (pivot_row, below) = remainder.split_first_mut().expect("pivot row exists");
            for value in &mut *pivot_row {
                *value /= pivot;
            }
            for row in above.iter_mut().chain(below.iter_mut()) {
                let coefficient = row[column];
                if coefficient.abs() > EPSILON {
                    for (value, pivot) in row.iter_mut().zip(pivot_row.iter()) {
                        *value -= coefficient * pivot;
                    }
                }
            }
        }
        table[rows][rhs]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moving_hauler_reuses_pickup_access_when_its_cell_center_is_blocked() {
        let grid = IntentGrid::new(8, 8);
        let navigation =
            Navigation::new(&grid, vec![Obstacle::deposit(Vec2::new(30.0, 36.0), 3.0)]);
        let footprint = Obstacle::deposit(Vec2::splat(400.0), 40.0);
        let pickup = Vec2::new(-180.0, 36.0);
        let original = Vec2::new(70.0, 18.0);
        let moved = Vec2::new(70.0, 21.0);
        let mut paths = DeliveryPaths::default();
        paths.begin(&navigation);

        assert!(!navigation.point_clear_for_entry(Vec2::splat(36.0)));
        assert!(delivery_clear(&navigation, footprint, original, moved));
        for position in [original, moved] {
            assert!(!delivery_clear(&navigation, footprint, position, pickup));
        }

        let mut estimate = paths.estimate_pickup(&navigation, footprint, original, pickup);
        for _ in 0..20 {
            if !matches!(estimate, RouteEstimate::Connecting) {
                break;
            }
            navigation.advance(&grid, 4096);
            estimate = paths.estimate_pickup(&navigation, footprint, original, pickup);
        }
        let RouteEstimate::Pending(probe) = estimate else {
            panic!("the original pickup approach should need a budgeted route");
        };
        paths.request(&navigation, footprint, probe);

        assert!(matches!(
            paths.estimate_pickup(&navigation, footprint, moved, pickup),
            RouteEstimate::Pending(probe) if probe.start == original
        ));
        for _ in 0..20 {
            navigation.advance(&grid, 4096);
            paths.begin(&navigation);
            if paths.pending.is_none() {
                break;
            }
        }
        assert!(matches!(
            paths.estimate_pickup(&navigation, footprint, moved, pickup),
            RouteEstimate::Known(0.0)
        ));
    }
}
