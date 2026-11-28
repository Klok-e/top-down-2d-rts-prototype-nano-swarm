//! Rolling Worker replenishment and uncommitted Hauler time for facility expansion.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::{ecs::system::SystemParam, prelude::*};

use crate::{
    intent::IntentGrid,
    navigation::{
        ConnectivityStatus, Navigation, Obstacle, RouteGoal, RouteRequestId, RouteStatus, ShapeBits,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
    tuning::Tuning,
};

use super::{
    Cargo, Charger, HaulerAssignment, HaulerLoading, Health, InteractionRegion, Nanobot,
    NanobotType, OwnerSwarm, SupportCondition, Swarm, SwarmId, SwarmMember, spending_capacity,
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
        let transfer = logistics.hauler_transfer_per_pulse.max(1);
        let work = f64::from(logistics.material_work_ticks);
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
                            region: InteractionRegion::structure(transform),
                            minerals,
                            load: f64::from(load),
                            transfer_ticks_per_mineral: 2.0
                                * f64::from(load.div_ceil(transfer))
                                * work
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
        let region = InteractionRegion::structure(site);
        let footprint = Obstacle::structure(site);
        let mut confirmed = Vec::new();
        let mut possible = Vec::new();
        let mut pending = budget
            .sources
            .iter()
            .any(|source| !source.pending_spending.is_empty());
        for (index, source) in budget.sources.iter().enumerate() {
            let (distance, pickup) =
                match self
                    .paths
                    .round_trip(&self.navigation, region, footprint, source)
                {
                    RouteEstimate::Known(route) => route,
                    RouteEstimate::Missing => continue,
                    RouteEstimate::Pending => {
                        pending = true;
                        // Even free travel cannot exceed the loading/unloading bound.
                        possible.push(HaulRoute {
                            source: index,
                            ticks_per_mineral: source.transfer_ticks_per_mineral,
                            haulers: (0..budget.haulers.len()).collect(),
                        });
                        continue;
                    }
                };
            let ticks_per_mineral =
                distance / budget.speed / source.load + source.transfer_ticks_per_mineral;
            let mut confirmed_haulers = Vec::new();
            let mut possible_haulers = Vec::new();
            for (hauler, &(position, _)) in budget.haulers.iter().enumerate() {
                match connects(&self.navigation, position, RouteGoal::Point(pickup), pickup) {
                    Some(true) => {
                        confirmed_haulers.push(hauler);
                        possible_haulers.push(hauler);
                    }
                    Some(false) => {}
                    None => {
                        pending = true;
                        possible_haulers.push(hauler);
                    }
                }
            }
            confirmed.push(HaulRoute {
                source: index,
                ticks_per_mineral,
                haulers: confirmed_haulers,
            });
            possible.push(HaulRoute {
                source: index,
                ticks_per_mineral,
                haulers: possible_haulers,
            });
        }

        if budget.covers_site(&confirmed, false) {
            Some(true)
        } else if pending && budget.covers_site(&possible, true) {
            None
        } else {
            Some(false)
        }
    }
}

struct HaulRoute {
    source: usize,
    ticks_per_mineral: f64,
    haulers: Vec<usize>,
}

impl SupplyBudget {
    fn covers_site(&self, routes: &[HaulRoute], include_pending: bool) -> bool {
        if !covers(
            routes
                .iter()
                .filter(|route| !route.haulers.is_empty())
                .map(|route| self.sources[route.source].minerals)
                .sum(),
            self.expansion,
        ) {
            return false;
        }

        // Interchangeable Haulers share one idle-time budget for their set of usable routes.
        let mut pools = BTreeMap::<Vec<usize>, u64>::new();
        for (hauler, &(_, time)) in self.haulers.iter().enumerate() {
            let reachable = routes
                .iter()
                .enumerate()
                .filter_map(|(index, route)| {
                    route
                        .haulers
                        .binary_search(&hauler)
                        .is_ok()
                        .then_some(index)
                })
                .collect::<Vec<_>>();
            if !reachable.is_empty() {
                *pools.entry(reachable).or_default() += u64::from(time);
            }
        }
        let idle_capacity = pools
            .iter()
            .map(|(reachable, time)| {
                let fastest = reachable
                    .iter()
                    .map(|&index| routes[index].ticks_per_mineral)
                    .min_by(f64::total_cmp)
                    .expect("pool serves a route");
                *time as f64 / fastest
            })
            .sum();
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
            for route in reachable {
                let route = &routes[route];
                flow.allow(&[
                    (route.source, 1.0),
                    (expansion_row, 1.0),
                    (expansion_row + 1 + index, route.ticks_per_mineral),
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

type PathKey = (ShapeBits, [u32; 2], super::interaction::InteractionBits);
type RoundTrip = (f64, Vec2);

enum RouteEstimate {
    Known(RoundTrip),
    Pending,
    Missing,
}

#[derive(Default)]
struct DeliveryPaths {
    geometry: Option<(u64, u64)>,
    pending: HashMap<PathKey, RouteRequestId>,
    finished: HashMap<PathKey, Option<RoundTrip>>,
}

impl DeliveryPaths {
    fn begin(&mut self, navigation: &Navigation) {
        let geometry = (navigation.clearance_identity(), navigation.revision());
        if self.geometry != Some(geometry) {
            for id in self.pending.drain().map(|(_, id)| id) {
                navigation.cancel(id);
            }
            self.finished.clear();
            self.geometry = Some(geometry);
            return;
        }
        self.pending.retain(|key, id| {
            let result = match navigation.poll(*id) {
                RouteStatus::Pending => return true,
                RouteStatus::Unreachable => None,
                RouteStatus::Found(route) => route.waypoints.last().copied().map(|pickup| {
                    // Forecast routes use physical distance without Corridor or traffic costs.
                    (2.0 * f64::from(route.cost), pickup)
                }),
            };
            navigation.cancel(*id);
            self.finished.insert(*key, result);
            false
        });
    }

    /// One repeated delivery from a Source, including the empty return along the same path.
    fn round_trip(
        &mut self,
        navigation: &Navigation,
        destination: InteractionRegion,
        footprint: Obstacle,
        source: &Source,
    ) -> RouteEstimate {
        let best = destination.approach(source.center);
        let direct = |start| {
            let pickup = source.region.approach(start);
            (footprint.segment_clear(start, pickup)
                && navigation.point_clear_for_entry(start)
                && navigation.movement_clear(start, pickup))
            .then_some((2.0 * f64::from(start.distance(pickup)), pickup))
        };
        if let Some(route) = direct(best) {
            return RouteEstimate::Known(route);
        }
        for start in std::iter::once(best).chain(destination.candidates(source.center)) {
            if !navigation.point_clear_for_entry(start) {
                continue;
            }
            if let Some(route) = direct(start) {
                return RouteEstimate::Known(route);
            }
            let key = (
                footprint.bits(),
                start.to_array().map(f32::to_bits),
                source.region.bits(),
            );
            if let Some(result) = self.finished.get(&key) {
                if let Some(route) = result {
                    return RouteEstimate::Known(*route);
                }
                continue;
            }
            let goal = RouteGoal::Interaction(source.region);
            if matches!(
                navigation.query_connectivity(start, goal),
                ConnectivityStatus::Unreachable
            ) {
                continue;
            }
            // Share the normal background budget and keep at most one forecast queued.
            if self.pending.is_empty() {
                let id = navigation.request_delivery_route(start, goal, footprint);
                self.pending.insert(key, id);
            }
            return RouteEstimate::Pending;
        }
        RouteEstimate::Missing
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
