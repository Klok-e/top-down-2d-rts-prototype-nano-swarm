use super::{
    lifecycle::WorkBudget,
    materialize::{
        CorridorPaintLimit, cell_is_reachable, corridor_route, intent_cell_center,
        retain_build_footprints, route_exposure, world_to_intent_cell,
    },
    *,
};
use crate::nanobot::{
    HAUL_WINDOW_TICKS, InteractionRegion, cell_overlaps_circle, deposit_candidate_cells,
    estimated_lead_ticks, throughput_quota,
};
use bevy::prelude::Transform;
use std::cmp::Reverse;

// Leave room for the home base and mandatory military support.
pub(super) const SUPPORT_PAINT_RESERVE: usize = 8;
const ECONOMY_EXPANSION_PAINT_LIMIT: usize = EXPANSION_PLAN_CELLS - SUPPORT_PAINT_RESERVE;
const ECONOMY_RETENTION_PAINT_LIMIT: usize = MAX_PLAN_CELLS - SUPPORT_PAINT_RESERVE;
#[derive(Debug, Clone, PartialEq)]
pub(super) struct OwnedDeposit {
    pub(super) id: u64,
    position: Vec2,
    haul_distance: f32,
    pub(super) gather_cell: IVec2,
    pub(super) build_cell: IVec2,
    desired: Vec<DesiredIntent>,
}

#[derive(Default)]
pub(super) struct EconomyPlan {
    pub(super) owned_deposits: Vec<OwnedDeposit>,
    pub(super) desired: HashSet<DesiredIntent>,
    pub(super) expansion: Option<u64>,
    pub(super) threatened: Option<(u64, IVec2)>,
    pub(super) free_deposit_exists: bool,
    pub(super) contested_deposits: Vec<(u64, IVec2, RouteGoal)>,
    pub(super) enemy_deposits: Vec<(DepositState, SwarmId)>,
    pub(super) enemy_gather_claims: BTreeSet<(u64, SwarmId)>,
}

impl EconomyPlan {
    fn can_fit(&self, mine: &OwnedDeposit, limit: usize, budget: &mut WorkBudget) -> bool {
        let Some(mut room) = limit.checked_sub(self.desired.len()) else {
            return false;
        };
        if mine.desired.len() <= room {
            return true;
        }
        for intent in &mine.desired {
            if !budget.spend(1) {
                return false;
            }
            if !self.desired.contains(intent) {
                let Some(remaining) = room.checked_sub(1) else {
                    return false;
                };
                room = remaining;
            }
        }
        true
    }

    fn add(&mut self, mine: OwnedDeposit, budget: &mut WorkBudget) -> bool {
        if !budget.spend(mine.desired.len()) {
            return false;
        }
        self.desired.extend(mine.desired.iter().copied());
        self.owned_deposits.push(mine);
        true
    }
}

fn available(deposit: &DepositState, owner: SwarmId) -> bool {
    deposit.amount > 0 && deposit.owner.is_none_or(|exclusive| exclusive == owner)
}

pub(super) fn validate_owned_deposits(
    owned: &[OwnedDeposit],
    owner: SwarmId,
    state: &GameState<'_>,
    unreachable: &mut HashSet<DesiredIntent>,
    budget: &mut WorkBudget,
) -> Result<Option<u64>, ()> {
    let mut lost = None;
    for mine in owned {
        let mut valid = true;
        if lost.is_none() {
            valid = find_deposit(mine.id, state, budget)?.is_some_and(|deposit| {
                available(deposit, owner) && deposit.position == mine.position
            });
        }
        if !unreachable.is_empty() {
            for intent in &mine.desired {
                if !budget.spend(1) {
                    return Err(());
                }
                if unreachable.remove(intent) {
                    valid = false;
                    if unreachable.is_empty() {
                        break;
                    }
                }
            }
        }
        if !valid {
            lost.get_or_insert(mine.id);
        }
        if lost.is_some() && unreachable.is_empty() {
            break;
        }
    }
    Ok(lost)
}

fn find_deposit<'a>(
    id: u64,
    state: &'a GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<Option<&'a DepositState>, ()> {
    for deposit in state.deposits {
        if !budget.spend(1) {
            return Err(());
        }
        if deposit.id == id {
            return Ok(Some(deposit));
        }
    }
    Ok(None)
}

pub(super) fn plan_economy(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    previous: &[OwnedDeposit],
    budget: &mut WorkBudget,
) -> Option<EconomyPlan> {
    if !budget.spend(
        state
            .swarms
            .len()
            .saturating_mul(2)
            .saturating_add(previous.len()),
    ) {
        return None;
    }
    let home = state
        .swarms
        .iter()
        .find(|swarm| swarm.id == owner)
        .map_or(Vec2::ZERO, |swarm| swarm.home);
    let enemy_home = state
        .swarms
        .iter()
        .filter(|swarm| swarm.id != owner)
        .min_by(|a, b| {
            home.distance_squared(a.home)
                .total_cmp(&home.distance_squared(b.home))
        })
        .map(|swarm| swarm.home);
    let mut economy = EconomyPlan {
        desired: retain_build_footprints(owner, state, ECONOMY_RETENTION_PAINT_LIMIT, budget)?,
        ..Default::default()
    };
    for mine in previous {
        let Some(deposit) = find_deposit(mine.id, state, budget)
            .ok()?
            .filter(|d| available(d, owner))
        else {
            continue;
        };
        match mine_support(owner, deposit, state, home, budget).ok()? {
            Some((supported, _)) => {
                if !economy.can_fit(&supported, ECONOMY_RETENTION_PAINT_LIMIT, budget)
                    || !economy.add(supported, budget)
                {
                    return None;
                }
            }
            None if budget.exhausted => return None,
            None => {}
        }
    }

    let replacing_lost_mine = economy.owned_deposits.len() < previous.len();
    let mut opportunities = Vec::new();

    let previous_ids: HashSet<_> = previous.iter().map(|mine| mine.id).collect();
    for deposit in state.deposits {
        if !budget.spend(1) {
            return None;
        }
        if deposit.amount == 0 || (available(deposit, owner) && previous_ids.contains(&deposit.id))
        {
            continue;
        }
        let (reachable, gather_owners) = deposit_access(owner, deposit, state, budget)?;
        if !reachable {
            continue;
        }
        if let Some(other) = deposit.owner.filter(|other| *other != owner) {
            economy.enemy_deposits.push((*deposit, other));
            if gather_owners.contains(&other) {
                economy.enemy_gather_claims.insert((deposit.id, other));
            }
            continue;
        }
        if gather_owners.contains(&owner) || deposit.owner == Some(owner) {
            if let Ok(Some((mine, _))) = mine_support(owner, deposit, state, home, budget)
                && economy.can_fit(&mine, ECONOMY_EXPANSION_PAINT_LIMIT, budget)
                && !economy.add(mine, budget)
            {
                return None;
            }
            if budget.exhausted {
                return None;
            }
            continue;
        }
        if !gather_owners.is_empty() {
            for other in gather_owners {
                if !budget.spend(1) {
                    return None;
                }
                economy.enemy_deposits.push((*deposit, other));
                economy.enemy_gather_claims.insert((deposit.id, other));
            }
            continue;
        }
        opportunities.push(deposit);
    }
    if budget.exhausted {
        return None;
    }

    economy.threatened = threatened_deposit(&economy.owned_deposits, owner, state, budget)?;
    if economy.threatened.is_some() {
        return Some(economy);
    }

    let mut unchecked = Vec::new();
    for deposit in opportunities {
        let mut approached = false;
        for bot in state.bots {
            if !budget.spend(1) {
                return None;
            }
            if bot.owner != owner
                && bot.health > 0
                && approaches(bot, deposit)
                && reachable_position(owner, bot.position, state, budget)?
            {
                approached = true;
                break;
            }
        }
        if approached {
            match mine_support(owner, deposit, state, home, budget) {
                Ok(Some((mine, _))) => {
                    economy.contested_deposits.push((
                        deposit.id,
                        mine.gather_cell,
                        RouteGoal::Interaction(
                            deposit_region(deposit).within_cell(mine.gather_cell),
                        ),
                    ));
                }
                Err(_) => economy.free_deposit_exists = true,
                Ok(None) => {}
            }
        } else {
            unchecked.push(deposit);
        }
        if budget.exhausted {
            return None;
        }
    }
    if unchecked.is_empty() {
        return Some(economy);
    }
    let establishing_supply = economy.owned_deposits.is_empty() || replacing_lost_mine;
    let observation = state
        .supply
        .and_then(|supply| supply.economies.get(&owner))
        .copied()
        .unwrap_or_default();
    if !establishing_supply && observation.low_reserve_ticks < HAUL_WINDOW_TICKS {
        return Some(economy);
    }
    let mut retained = Vec::with_capacity(economy.owned_deposits.len());
    for mine in &economy.owned_deposits {
        let deposit = find_deposit(mine.id, state, budget).ok()??;
        let mine = mine_observation(owner, deposit, mine.gather_cell, state, budget)?;
        if !establishing_supply && !mine.is_saturated() {
            return Some(economy);
        }
        retained.push(mine);
    }
    if !establishing_supply && !needs_more_extraction(observation, &retained) {
        return Some(economy);
    }
    let capacity = expansion_capacity(owner, state, observation, &retained, budget)?;
    if capacity.workers == 0 || capacity.haulers == 0 {
        return Some(economy);
    }
    let haul_overhead = if economy.owned_deposits.len() > 1 {
        if !budget.spend(economy.owned_deposits.len()) {
            return None;
        }
        economy
            .owned_deposits
            .iter()
            .map(|mine| mine.haul_distance)
            .sum::<f32>()
            / capacity.total_haulers as f32
    } else {
        0.0
    };

    // Start with the cheapest plausible routes so distant candidates usually
    // need only an optimistic value bound, even when the map has many deposits.
    let opportunities_count = unchecked.len();
    if opportunities_count > 1
        && !budget.spend(opportunities_count.saturating_mul(opportunities_count.ilog2() as usize))
    {
        return None;
    }
    unchecked.sort_by(|a, b| {
        home.distance_squared(a.position)
            .total_cmp(&home.distance_squared(b.position))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut best: Option<(OwnedDeposit, f32)> = None;
    for deposit in unchecked {
        let terrain_exposure = route_exposure(home, deposit.position, state.terrain, budget);
        if budget.exhausted {
            return None;
        }
        let enemy_exposure = enemy_home.map_or(0.0, |enemy| {
            let friendly_distance = home.distance(deposit.position);
            let enemy_distance = enemy.distance(deposit.position);
            friendly_distance / (friendly_distance + enemy_distance).max(crate::ZONE_BLOCK_SIZE)
        });
        let value = (deposit.amount as f32 + 1.0).ln() * tuning.strategy_expansion_income_weight
            - (terrain_exposure + enemy_exposure) * tuning.strategy_expansion_exposure_cost;
        let travel = home.distance(deposit.position);
        let value = value
            - travel / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_travel_cost
            - haul_overhead / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_haul_cost;
        // Navigation can move an obstructed origin to clear ground in its tile.
        let origin_offset = if state.route_query.is_some() {
            Vec2::splat(crate::ZONE_BLOCK_SIZE).length()
        } else {
            (home - intent_cell_center(world_to_intent_cell(home))).length()
        };
        let min_route = (travel
            - origin_offset
            - deposit.radius
            - crate::navigation::BODY_RADIUS
            - 2.0 * crate::nanobot::STOP_THRESHOLD)
            .max(0.0);
        let optimistic =
            value - min_route / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_haul_cost;
        let could_claim = optimistic > best.as_ref().map_or(0.0, |(_, score)| *score);
        if !could_claim && economy.free_deposit_exists {
            continue;
        }
        let (mine, distance) = match mine_support(owner, deposit, state, home, budget) {
            Ok(Some(support)) => support,
            Ok(None) if budget.exhausted => return None,
            Ok(None) => continue,
            Err(_) => {
                economy.free_deposit_exists = true;
                continue;
            }
        };
        economy.free_deposit_exists = true;
        let score = value - distance / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_haul_cost;
        if score > best.as_ref().map_or(0.0, |(_, score)| *score)
            && economy.can_fit(&mine, ECONOMY_EXPANSION_PAINT_LIMIT, budget)
            && capacity.can_deliver(state.supply.map(|supply| supply.logistics), distance)
            && (capacity.workers > 1
                || mine_observation(owner, deposit, mine.gather_cell, state, budget)?.has_source)
        {
            best = Some((mine, score));
        }
    }
    if let Some((mine, _)) = best {
        let id = mine.id;
        if !economy.add(mine, budget) {
            return None;
        }
        economy.expansion = Some(id);
    }
    (!budget.exhausted).then_some(economy)
}

fn mine_supply(mines: &[MineObservation]) -> f32 {
    mines
        .iter()
        .filter(|mine| mine.observation_ticks > 0)
        .map(|mine| {
            mine.delivered_minerals as f32 * crate::SIMULATION_HZ as f32
                / mine.observation_ticks as f32
        })
        .sum()
}

fn needs_more_extraction(observation: EconomyObservation, retained: &[MineObservation]) -> bool {
    let Some(haul) = observation.haul else {
        return false;
    };
    let supply = mine_supply(retained);
    // Mine activity and Haul Quotas use different windows. Allow a small
    // mismatch from loads and terminal consumption crossing a window boundary.
    if haul.haul_rate <= 0.0 || haul.delivered_rate < 0.75 * supply {
        return false;
    }
    let spending = observation
        .gather
        .spending_capacity
        .min(haul.spending_capacity);
    let observation_seconds = HAUL_WINDOW_TICKS as f32 / crate::SIMULATION_HZ as f32;
    (spending - supply.max(haul.delivered_rate)) * observation_seconds >= 1.0
}

pub(super) struct ExpansionCapacity {
    workers: u32,
    haulers: u32,
    total_haulers: u32,
    worker_rate: f32,
    measured_haul_rate: Option<f32>,
}

impl ExpansionCapacity {
    pub(super) fn can_deliver(&self, logistics: Option<&LogisticsTuning>, distance: f32) -> bool {
        let Some(logistics) = logistics else {
            return true;
        };
        let haul_rate = logistics.hauler_carry_capacity.max(1) as f32 * crate::SIMULATION_HZ as f32
            / estimated_lead_ticks(logistics, distance).max(1.0);
        let haul_rate = self
            .measured_haul_rate
            .map_or(haul_rate, |measured| haul_rate.min(measured));
        haul_rate * self.haulers as f32 >= self.worker_rate
    }
}

fn reachable_position(
    owner: SwarmId,
    position: Vec2,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<bool> {
    state.reachability.map_or(Some(true), |map| {
        map.position_is_reachable_with_budget(owner, position, state.grid, &mut || budget.spend(1))
            .ok()
    })
}

pub(super) fn expansion_capacity(
    owner: SwarmId,
    state: &GameState<'_>,
    observation: EconomyObservation,
    retained: &[MineObservation],
    budget: &mut WorkBudget,
) -> Option<ExpansionCapacity> {
    let mut workers = 0u32;
    let mut haulers = 0u32;
    for bot in state.bots {
        if !budget.spend(1) {
            return None;
        }
        if bot.owner != owner
            || bot.health == 0
            || bot.kind == NanobotType::Defender
            || !bot.position.is_finite()
            || !state.grid.in_bounds(world_to_intent_cell(bot.position))
            || !reachable_position(owner, bot.position, state, budget)?
        {
            continue;
        }
        match bot.kind {
            NanobotType::Worker => workers += 1,
            NanobotType::Hauler => haulers += 1,
            NanobotType::Defender => {}
        }
    }
    if !budget.spend(retained.len().saturating_mul(2)) {
        return None;
    }
    let mine_workers = retained.iter().fold(0u32, |workers, mine| {
        workers.saturating_add(mine.active_workers.max(mine.worker_capacity))
    });
    let gathering = mine_workers.max(observation.gather.gathering_workers);
    let haul_commitments = observation.haul.map_or(0, |haul| {
        let existing_flow = mine_supply(retained).max(haul.delivered_rate);
        haul.haul_quota
            .max(throughput_quota(existing_flow, haul.haul_rate))
    });
    Some(ExpansionCapacity {
        workers: workers.saturating_sub(gathering.saturating_add(observation.upkeep_workers)),
        haulers: haulers.saturating_sub(haul_commitments.max(observation.committed_haulers)),
        total_haulers: haulers,
        worker_rate: observation.gather.gather_rate,
        measured_haul_rate: observation
            .haul
            .map(|haul| haul.haul_rate)
            .filter(|rate| *rate > 0.0),
    })
}

pub(super) fn mine_observation(
    owner: SwarmId,
    deposit: &DepositState,
    gather_cell: IVec2,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<MineObservation> {
    state
        .mine_query
        .map_or(Some(MineObservation::default()), |query| {
            query(
                owner,
                deposit,
                MineObservationAt::Service(gather_cell),
                &mut || budget.spend(1),
            )
        })
}

fn approaches(bot: &BotState, deposit: &DepositState) -> bool {
    let offset = deposit.position - bot.position;
    offset.length_squared() <= (2.0 * crate::ZONE_BLOCK_SIZE).powi(2)
        && bot.displacement.length_squared() >= 0.25
        && bot
            .displacement
            .normalize_or_zero()
            .dot(offset.normalize_or_zero())
            >= 0.8
}

fn threatened_deposit(
    owned: &[OwnedDeposit],
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<Option<(u64, IVec2)>> {
    const DROPOFF_GRACE_TICKS: u64 = 3 * crate::SIMULATION_HZ as u64;
    const DISRUPTION_TICKS: u64 = 2 * DROPOFF_GRACE_TICKS;

    let Some(query) = state.mine_query else {
        return Some(None);
    };
    let mut most_threatened = None;
    for mine in owned {
        let deposit = find_deposit(mine.id, state, budget).ok()??;
        let cells = [mine.gather_cell, mine.build_cell];
        let observation = query(
            owner,
            deposit,
            MineObservationAt::Defense(cells),
            &mut || budget.spend(1),
        )?;
        let pressure = observation.pressure;
        let damage = pressure.recent_damage.into_iter().sum::<u64>();
        let hostile_denial =
            pressure
                .ticks_since_attack_during_presence
                .is_some_and(|since_attack| {
                    observation
                        .ticks_since_dropoff
                        .is_none_or(|since_delivery| since_delivery >= since_attack)
                });
        let disrupted = hostile_denial
            || pressure.armed_presence_ticks >= DISRUPTION_TICKS
                && observation.ticks_since_dropoff.is_some_and(|elapsed| {
                    elapsed >= DISRUPTION_TICKS
                        && elapsed <= pressure.armed_presence_ticks + DROPOFF_GRACE_TICKS
                });
        if damage == 0 && !disrupted {
            continue;
        }
        let danger = (
            pressure.ongoing_damage,
            disrupted,
            damage,
            pressure.armed_defenders,
        );
        let importance = (
            observation.delivered_minerals,
            observation.completed_dropoffs,
            observation.active_workers,
        );
        let rank = (danger, importance, Reverse(mine.id));
        if most_threatened.is_none_or(|(best, _)| rank > best) {
            let target = cells[usize::from(pressure.recent_damage[1] > pressure.recent_damage[0])];
            most_threatened = Some((rank, (mine.id, target)));
        }
    }
    Some(most_threatened.map(|(_, threat)| threat))
}

fn deposit_access(
    owner: SwarmId,
    deposit: &DepositState,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<(bool, BTreeSet<SwarmId>)> {
    let mut reachable = state.route_query.is_some();
    let mut gather_owners = BTreeSet::new();
    for cell in deposit_candidate_cells(state.grid, deposit.position, deposit.radius) {
        if !budget.spend(1) {
            return None;
        }
        if let Some(intent) = state.grid.cell(cell)
            && cell_overlaps_circle(cell, deposit.position, deposit.radius)
        {
            if !reachable {
                reachable = cell_is_reachable(owner, state, cell, budget)?;
            }
            if intent.has(IntentKind::Gather) {
                for other in intent.owners(IntentKind::Gather) {
                    if !budget.spend(1) {
                        return None;
                    }
                    gather_owners.insert(other);
                }
            }
        }
    }
    Some((reachable, gather_owners))
}

pub(super) fn deposit_region(deposit: &DepositState) -> InteractionRegion {
    InteractionRegion::deposit(
        &Transform::from_translation(deposit.position.extend(0.0)),
        deposit.radius,
    )
}

fn mine_support(
    owner: SwarmId,
    deposit: &DepositState,
    state: &GameState<'_>,
    home: Vec2,
    budget: &mut WorkBudget,
) -> Result<Option<(OwnedDeposit, f32)>, CorridorPaintLimit> {
    let center = world_to_intent_cell(deposit.position);
    let region = deposit_region(deposit);
    let cells = std::iter::once(center).chain(
        deposit_candidate_cells(state.grid, deposit.position, deposit.radius)
            .filter(|cell| *cell != center),
    );
    let mut exceeds_paint_limit = false;
    for gather_cell in cells {
        if !budget.spend(1) {
            return Ok(None);
        }
        if !state.grid.in_bounds(gather_cell)
            || !cell_overlaps_circle(gather_cell, deposit.position, deposit.radius)
        {
            continue;
        }
        let interaction = region.within_cell(gather_cell);
        if state.route_query.is_none()
            && let Some(map) = state.reachability
        {
            match map
                .interaction_is_reachable_with_budget(owner, interaction, &mut || budget.spend(1))
            {
                Ok(true) => {}
                Ok(false) => continue,
                Err(()) => return Ok(None),
            }
        }
        let goal = RouteGoal::Interaction(interaction);
        let (route, distance) = match corridor_route(owner, state, budget, home, gather_cell, goal)
        {
            Ok(Some(route)) => route,
            Ok(None) if budget.exhausted => return Ok(None),
            Ok(None) => continue,
            Err(_) => {
                exceeds_paint_limit = true;
                continue;
            }
        };
        if !budget.spend(route.len() + 4) {
            return Ok(None);
        }
        let viable = std::iter::once(gather_cell)
            .chain(route.iter().copied())
            .all(|cell| cell_is_reachable(owner, state, cell, budget) == Some(true));
        if budget.exhausted {
            return Ok(None);
        }
        if !viable {
            continue;
        }

        let support = [IVec2::X, -IVec2::X, IVec2::Y, -IVec2::Y];
        let mut support = support.map(|offset| gather_cell + offset);
        support.sort_by(|a, b| {
            home.distance_squared(intent_cell_center(*a))
                .total_cmp(&home.distance_squared(intent_cell_center(*b)))
        });
        let build_cell = support.into_iter().find(|&cell| {
            state
                .terrain
                .iter()
                .all(|rock| budget.spend(1) && rock.admits_body(intent_cell_center(cell)))
                && cell_is_reachable(owner, state, cell, budget) == Some(true)
        });
        if budget.exhausted {
            return Ok(None);
        }
        let Some(build_cell) = build_cell else {
            continue;
        };
        let mut desired = vec![
            DesiredIntent {
                cell: gather_cell,
                kind: IntentKind::Gather,
            },
            DesiredIntent {
                cell: build_cell,
                kind: IntentKind::Build,
            },
            DesiredIntent {
                cell: gather_cell,
                kind: IntentKind::Defend,
            },
            DesiredIntent {
                cell: build_cell,
                kind: IntentKind::Defend,
            },
        ];
        desired.extend(route.into_iter().map(|cell| DesiredIntent {
            cell,
            kind: IntentKind::Corridor,
        }));
        return Ok(Some((
            OwnedDeposit {
                id: deposit.id,
                position: deposit.position,
                haul_distance: distance,
                gather_cell,
                build_cell,
                desired,
            },
            distance,
        )));
    }
    if exceeds_paint_limit {
        Err(CorridorPaintLimit)
    } else {
        Ok(None)
    }
}
