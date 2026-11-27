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
    pub(super) desired: Vec<DesiredIntent>,
    blocked_since: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MineRecovery {
    pub(super) vacancies: usize,
    pub(super) retired: BTreeSet<u64>,
    pub(super) source_gather: Vec<IVec2>,
}

pub(super) struct MineClaim {
    pub(super) mine: OwnedDeposit,
    pub(super) replacement: Option<MineReplacement>,
}

pub(super) struct MineReplacement {
    pub(super) retires: u64,
    pub(super) retained_paint: HashSet<DesiredIntent>,
}

#[derive(Default)]
pub(super) struct EconomyPlan {
    pub(super) owned_deposits: Vec<OwnedDeposit>,
    pub(super) desired: HashSet<DesiredIntent>,
    pub(super) expansion: Option<MineClaim>,
    pub(super) recovery: MineRecovery,
    pub(super) threatened: Option<(u64, IVec2)>,
    pub(super) contested_deposits: Vec<(u64, IVec2, RouteGoal)>,
    pub(super) enemy_deposits: Vec<(DepositState, SwarmId)>,
    pub(super) enemy_gather_claims: BTreeSet<(u64, SwarmId)>,
}

impl EconomyPlan {
    fn add(&mut self, mine: OwnedDeposit, budget: &mut WorkBudget) -> bool {
        if !budget.spend(mine.desired.len()) {
            return false;
        }
        self.desired.extend(mine.desired.iter().copied());
        self.owned_deposits.push(mine);
        true
    }
}

fn can_fit(
    desired: &HashSet<DesiredIntent>,
    mine: &OwnedDeposit,
    limit: usize,
    budget: &mut WorkBudget,
) -> bool {
    let Some(mut room) = limit.checked_sub(desired.len()) else {
        return false;
    };
    if mine.desired.len() <= room {
        return true;
    }
    for intent in &mine.desired {
        if !budget.spend(1) {
            return false;
        }
        if !desired.contains(intent) {
            let Some(remaining) = room.checked_sub(1) else {
                return false;
            };
            room = remaining;
        }
    }
    true
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
    previous: Option<&IntentPlan>,
    budget: &mut WorkBudget,
) -> Option<EconomyPlan> {
    let previous_mines = previous.map_or(&[][..], |plan| plan.owned_deposits.as_slice());
    if !budget.spend(
        state
            .swarms
            .len()
            .saturating_mul(2)
            .saturating_add(previous_mines.len()),
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
    let mut structural_support =
        retain_build_footprints(owner, state, ECONOMY_RETENTION_PAINT_LIMIT, budget)?;
    if !budget.spend(structural_support.len()) {
        return None;
    }
    let mut economy = EconomyPlan {
        desired: structural_support.clone(),
        ..Default::default()
    };
    for mine in previous_mines {
        let Some(deposit) = find_deposit(mine.id, state, budget)
            .ok()?
            .filter(|d| available(d, owner))
        else {
            continue;
        };
        let mut supported = match mine_support_where(
            owner,
            deposit,
            state,
            home,
            Some(mine.gather_cell),
            budget,
            |candidate, budget| {
                if candidate.gather_cell == mine.gather_cell || deposit.position != mine.position {
                    Some(true)
                } else {
                    cell_is_reachable(owner, state, mine.gather_cell, budget)
                        .map(|reachable| !reachable)
                }
            },
        ) {
            Ok(Some((supported, _))) => supported,
            _ if budget.exhausted => return None,
            _ if deposit.position == mine.position
                && cell_is_reachable(owner, state, mine.gather_cell, budget)? =>
            {
                if !budget.spend(mine.desired.len()) {
                    return None;
                }
                let mut supported = mine.clone();
                supported.desired.retain(|intent| {
                    cell_is_reachable(owner, state, intent.cell, budget) == Some(true)
                });
                supported
            }
            _ => continue,
        };
        supported.blocked_since = mine.blocked_since.filter(|_| {
            supported.position == mine.position && supported.gather_cell == mine.gather_cell
        });
        if !can_fit(
            &economy.desired,
            &supported,
            ECONOMY_RETENTION_PAINT_LIMIT,
            budget,
        ) || !economy.add(supported, budget)
        {
            return None;
        }
    }

    let mut opportunities = Vec::new();
    let previous_ids: HashSet<_> = previous_mines.iter().map(|mine| mine.id).collect();
    for deposit in state.deposits {
        if !budget.spend(1) {
            return None;
        }
        if deposit.amount == 0 {
            continue;
        }
        if previous.is_some_and(|plan| plan.mine_recovery.retired.contains(&deposit.id)) {
            economy.recovery.retired.insert(deposit.id);
            if available(deposit, owner) {
                continue;
            }
        }
        if available(deposit, owner) && previous_ids.contains(&deposit.id) {
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
                && can_fit(
                    &economy.desired,
                    &mine,
                    ECONOMY_EXPANSION_PAINT_LIMIT,
                    budget,
                )
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

    economy.recovery.vacancies = previous
        .map_or(0, |plan| plan.mine_recovery.vacancies)
        .saturating_add(previous_mines.len())
        .saturating_sub(economy.owned_deposits.len());

    if let Some(previous) = previous {
        economy.recovery.source_gather =
            retain_source_gather(owner, state, previous, &economy.owned_deposits, budget)?;
        if !budget.spend(2 * economy.recovery.source_gather.len()) {
            return None;
        }
        for &cell in &economy.recovery.source_gather {
            let paint = DesiredIntent {
                cell,
                kind: IntentKind::Gather,
            };
            structural_support.insert(paint);
            economy.desired.insert(paint);
        }
        if economy.desired.len() > ECONOMY_RETENTION_PAINT_LIMIT {
            return None;
        }
    }

    let mut observation = state
        .supply
        .and_then(|supply| supply.economies.get(&owner))
        .copied()
        .unwrap_or_default();
    let mut retained = BTreeMap::new();
    let mut failed_mine = None;
    let mut rerouted_any = false;
    for index in 0..economy.owned_deposits.len() {
        let other_gather = gather_support(
            &economy.owned_deposits,
            &economy.recovery.source_gather,
            Some(economy.owned_deposits[index].id),
            budget,
        )?;
        let mine = &economy.owned_deposits[index];
        let deposit = find_deposit(mine.id, state, budget).ok()??;
        let mut service =
            if previous.is_some() || observation.low_reserve_ticks >= HAUL_WINDOW_TICKS {
                mine_observation(
                    owner,
                    deposit,
                    mine.gather_cell,
                    &other_gather,
                    SourceCheck::Existing,
                    state,
                    budget,
                )?
            } else {
                MineObservation::default()
            };
        let mut blocked_since = (service.source_path != Some(true)).then(|| {
            let last_dropoff = service
                .ticks_since_dropoff
                .map_or(0, |elapsed| state.tick.saturating_sub(elapsed));
            mine.blocked_since
                .filter(|since| *since <= state.tick)
                .unwrap_or(state.tick)
                .max(last_dropoff)
        });
        let stalled =
            blocked_since.is_some_and(|since| state.tick - since >= MINE_OBSERVATION_TICKS);
        let mut rerouted_mine = None;
        if stalled
            && (!opportunities.is_empty()
                || deposit_candidate_cells(state.grid, deposit.position, deposit.radius)
                    .any(|cell| cell != mine.gather_cell))
        {
            if service.source_path.is_none() {
                service = mine_observation(
                    owner,
                    deposit,
                    mine.gather_cell,
                    &other_gather,
                    SourceCheck::Restore,
                    state,
                    budget,
                )?;
            }
            if service.source_path == Some(false) {
                let mut alternate_service = MineObservation::default();
                let alternate = mine_support_where(
                    owner,
                    deposit,
                    state,
                    home,
                    None,
                    budget,
                    |candidate, budget| {
                        let gather_cell = candidate.gather_cell;
                        if gather_cell == mine.gather_cell {
                            return Some(false);
                        }
                        alternate_service = mine_observation(
                            owner,
                            deposit,
                            gather_cell,
                            &other_gather,
                            SourceCheck::Restore,
                            state,
                            budget,
                        )?;
                        if alternate_service.source_path != Some(true) {
                            return Some(false);
                        }
                        if !budget.spend(other_gather.len() + 1) {
                            return None;
                        }
                        let mut proposed_gather = other_gather.clone();
                        proposed_gather.push(gather_cell);
                        let old_gather = gather_support(
                            &economy.owned_deposits,
                            &economy.recovery.source_gather,
                            None,
                            budget,
                        )?;
                        preserves_mine_sources(
                            owner,
                            state,
                            economy
                                .owned_deposits
                                .iter()
                                .filter(|other| other.id != mine.id),
                            SourceBaseline::Gather(&old_gather),
                            &proposed_gather,
                            budget,
                        )
                    },
                );
                if budget.exhausted {
                    return None;
                }
                if let Ok(Some((rerouted, _))) = alternate {
                    rerouted_mine = Some(rerouted);
                    service = alternate_service;
                    rerouted_any = true;
                } else if failed_mine.is_none() && economy.recovery.vacancies == 0 {
                    failed_mine = Some(mine.id);
                }
            }
            if service.source_path == Some(true) {
                blocked_since = None;
            }
        }
        retained.insert(mine.id, service);
        let mine = &mut economy.owned_deposits[index];
        if let Some(rerouted) = rerouted_mine {
            *mine = rerouted;
        }
        mine.blocked_since = blocked_since;
    }

    if rerouted_any {
        if !budget.spend(structural_support.len()) {
            return None;
        }
        economy.desired = mine_paint(
            structural_support.clone(),
            &economy.owned_deposits,
            None,
            budget,
        )?;
    }
    economy.threatened = threatened_deposit(&economy.owned_deposits, owner, state, budget)?;

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
            if let Ok(Some((mine, _))) = mine_support(owner, deposit, state, home, budget) {
                economy.contested_deposits.push((
                    deposit.id,
                    mine.gather_cell,
                    RouteGoal::Interaction(deposit_region(deposit).within_cell(mine.gather_cell)),
                ));
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
    let recovering = economy.recovery.vacancies > 0 || failed_mine.is_some();
    let establishing_supply = economy.owned_deposits.is_empty() || recovering;
    if !establishing_supply && observation.low_reserve_ticks < HAUL_WINDOW_TICKS {
        return Some(economy);
    }
    let other_gather = gather_support(
        &economy.owned_deposits,
        &economy.recovery.source_gather,
        failed_mine,
        budget,
    )?;
    let mut workers_released_at = None;
    if let Some(id) = failed_mine
        && let Some(service) = retained.remove(&id)
        && service.unladen_gatherers > 0
    {
        let deposit = find_deposit(id, state, budget).ok()??;
        if !budget.spend(other_gather.len()) {
            return None;
        }
        if other_gather
            .iter()
            .all(|&cell| !cell_overlaps_circle(cell, deposit.position, deposit.radius))
        {
            workers_released_at = Some(deposit);
            observation.gather.gathering_workers = observation
                .gather
                .gathering_workers
                .saturating_sub(service.unladen_gatherers);
        }
    }
    let retained_services: Vec<_> = retained.values().copied().collect();
    if !establishing_supply
        && (retained_services
            .iter()
            .any(|mine| mine.source_path == Some(false) || !mine.is_saturated())
            || !needs_more_extraction(observation, &retained_services))
    {
        return Some(economy);
    }
    let capacity = expansion_capacity(owner, state, observation, &retained_services, budget)?;
    if capacity.workers == 0 || capacity.haulers == 0 {
        return Some(economy);
    }
    let haul_overhead = if retained.len() > 1 {
        if !budget.spend(economy.owned_deposits.len()) {
            return None;
        }
        economy
            .owned_deposits
            .iter()
            .filter(|mine| Some(mine.id) != failed_mine)
            .map(|mine| mine.haul_distance)
            .sum::<f32>()
            / capacity.total_haulers as f32
    } else {
        0.0
    };

    let replacement = if let Some(retires) = failed_mine {
        Some(MineReplacement {
            retires,
            retained_paint: mine_paint(
                structural_support,
                &economy.owned_deposits,
                Some(retires),
                budget,
            )?,
        })
    } else {
        None
    };
    let retained_paint = replacement
        .as_ref()
        .map_or(&economy.desired, |replacement| &replacement.retained_paint);

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
        if optimistic <= best.as_ref().map_or(0.0, |(_, score)| *score) {
            continue;
        }
        let support = mine_support_where(
            owner,
            deposit,
            state,
            home,
            None,
            budget,
            |candidate, budget| {
                let score = value
                    - candidate.haul_distance / crate::ZONE_BLOCK_SIZE
                        * tuning.strategy_expansion_haul_cost;
                if score <= best.as_ref().map_or(0.0, |(_, best_score)| *best_score)
                    || !capacity.can_deliver(
                        state.supply.map(|supply| supply.logistics),
                        candidate.haul_distance,
                    )
                    || !can_fit(
                        retained_paint,
                        candidate,
                        ECONOMY_EXPANSION_PAINT_LIMIT,
                        budget,
                    )
                    || workers_released_at.is_some_and(|site| {
                        cell_overlaps_circle(candidate.gather_cell, site.position, site.radius)
                    })
                {
                    return Some(false);
                }
                if state.mine_query.is_none() {
                    return Some(capacity.workers > 1);
                }
                let service = mine_observation(
                    owner,
                    deposit,
                    candidate.gather_cell,
                    &other_gather,
                    SourceCheck::Claim {
                        allow_construction: capacity.workers > 1,
                    },
                    state,
                    budget,
                )?;
                Some(service.source_path == Some(true))
            },
        );
        let (mine, distance) = match support {
            Ok(Some(support)) => support,
            Ok(None) if budget.exhausted => return None,
            Ok(None) => continue,
            Err(_) => continue,
        };
        let score = value - distance / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_haul_cost;
        best = Some((mine, score));
    }
    if let Some((mine, _)) = best {
        if let Some(replacement) = &replacement
            && !preserves_mine_sources(
                owner,
                state,
                economy
                    .owned_deposits
                    .iter()
                    .filter(|mine| mine.id != replacement.retires),
                SourceBaseline::Observed(&retained),
                &other_gather,
                budget,
            )?
        {
            return Some(economy);
        }
        economy.expansion = Some(MineClaim { mine, replacement });
    }
    (!budget.exhausted).then_some(economy)
}

fn retain_source_gather(
    owner: SwarmId,
    state: &GameState<'_>,
    previous: &IntentPlan,
    mines: &[OwnedDeposit],
    budget: &mut WorkBudget,
) -> Option<Vec<IVec2>> {
    if mines.is_empty() {
        return Some(Vec::new());
    }
    let mut gather = gather_support(mines, &[], None, budget)?;
    let active = gather.len();
    if !budget.spend(active) {
        return None;
    }
    let mut seen: HashSet<_> = gather.iter().copied().collect();
    for cell in previous
        .mine_recovery
        .source_gather
        .iter()
        .copied()
        .chain(previous.owned_deposits.iter().map(|mine| mine.gather_cell))
    {
        if !budget.spend(1) {
            return None;
        }
        if seen.insert(cell) && cell_is_reachable(owner, state, cell, budget)? {
            gather.push(cell);
        }
    }
    if gather.len() == active {
        return Some(Vec::new());
    }
    let mut baseline = BTreeMap::new();
    for mine in mines {
        if !budget.spend(1) {
            return None;
        }
        let deposit = find_deposit(mine.id, state, budget).ok()??;
        baseline.insert(
            mine.id,
            mine_observation(
                owner,
                deposit,
                mine.gather_cell,
                &gather,
                SourceCheck::Existing,
                state,
                budget,
            )?,
        );
    }
    let mut index = active;
    while index < gather.len() {
        let cell = gather.swap_remove(index);
        if !preserves_mine_sources(
            owner,
            state,
            mines.iter(),
            SourceBaseline::Observed(&baseline),
            &gather,
            budget,
        )? {
            gather.push(cell);
            let last = gather.len() - 1;
            gather.swap(index, last);
            index += 1;
        }
    }
    Some(gather.split_off(active))
}

enum SourceBaseline<'a> {
    Observed(&'a BTreeMap<u64, MineObservation>),
    Gather(&'a [IVec2]),
}

fn preserves_mine_sources<'a>(
    owner: SwarmId,
    state: &GameState<'_>,
    mines: impl Iterator<Item = &'a OwnedDeposit>,
    baseline: SourceBaseline<'_>,
    gather: &[IVec2],
    budget: &mut WorkBudget,
) -> Option<bool> {
    for mine in mines {
        if !budget.spend(1) {
            return None;
        }
        let deposit = find_deposit(mine.id, state, budget).ok()??;
        let service = match baseline {
            SourceBaseline::Observed(observations) => *observations.get(&mine.id)?,
            SourceBaseline::Gather(old_gather) => mine_observation(
                owner,
                deposit,
                mine.gather_cell,
                old_gather,
                SourceCheck::Existing,
                state,
                budget,
            )?,
        };
        if service.source_path != Some(true) && service.covered_sources == 0 {
            continue;
        }
        let projected = mine_observation(
            owner,
            deposit,
            mine.gather_cell,
            gather,
            SourceCheck::Existing,
            state,
            budget,
        )?;
        let preserves_service = if service.source_path == Some(true) {
            projected.source_path == Some(true)
                && projected.worker_capacity >= service.worker_capacity
        } else {
            projected.source_path == Some(true)
                || projected.covered_sources >= service.covered_sources
        };
        if !preserves_service {
            return Some(false);
        }
    }
    Some(true)
}

fn mine_paint(
    mut desired: HashSet<DesiredIntent>,
    mines: &[OwnedDeposit],
    retired: Option<u64>,
    budget: &mut WorkBudget,
) -> Option<HashSet<DesiredIntent>> {
    for mine in mines.iter().filter(|mine| Some(mine.id) != retired) {
        if !budget.spend(mine.desired.len() + 1) {
            return None;
        }
        desired.extend(mine.desired.iter().copied());
    }
    (desired.len() <= ECONOMY_RETENTION_PAINT_LIMIT).then_some(desired)
}

fn gather_support(
    mines: &[OwnedDeposit],
    source_gather: &[IVec2],
    excluded: Option<u64>,
    budget: &mut WorkBudget,
) -> Option<Vec<IVec2>> {
    budget.spend(mines.len() + source_gather.len()).then(|| {
        mines
            .iter()
            .filter_map(|mine| (Some(mine.id) != excluded).then_some(mine.gather_cell))
            .chain(source_gather.iter().copied())
            .collect()
    })
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
    other_gather: &[IVec2],
    source_check: SourceCheck,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<MineObservation> {
    state
        .mine_query
        .map_or(Some(MineObservation::default()), |query| {
            query(
                owner,
                deposit,
                MineObservationAt::Service {
                    gather: gather_cell,
                    other_gather,
                    source_check,
                },
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
    mine_support_where(owner, deposit, state, home, None, budget, |_, _| Some(true))
}

fn mine_support_where(
    owner: SwarmId,
    deposit: &DepositState,
    state: &GameState<'_>,
    home: Vec2,
    preferred_gather: Option<IVec2>,
    budget: &mut WorkBudget,
    mut eligible: impl FnMut(&OwnedDeposit, &mut WorkBudget) -> Option<bool>,
) -> Result<Option<(OwnedDeposit, f32)>, CorridorPaintLimit> {
    let center = world_to_intent_cell(deposit.position);
    let region = deposit_region(deposit);
    let cells = preferred_gather
        .into_iter()
        .chain(std::iter::once(center).filter(|cell| Some(*cell) != preferred_gather))
        .chain(
            deposit_candidate_cells(state.grid, deposit.position, deposit.radius)
                .filter(|cell| *cell != center && Some(*cell) != preferred_gather),
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
        let mine = OwnedDeposit {
            id: deposit.id,
            position: deposit.position,
            haul_distance: distance,
            gather_cell,
            build_cell,
            desired,
            blocked_since: None,
        };
        match eligible(&mine, budget) {
            Some(true) => return Ok(Some((mine, distance))),
            Some(false) => continue,
            None => return Ok(None),
        }
    }
    if exceeds_paint_limit {
        Err(CorridorPaintLimit)
    } else {
        Ok(None)
    }
}
