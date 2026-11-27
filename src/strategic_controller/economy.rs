use super::{
    lifecycle::WorkBudget,
    materialize::{
        CorridorPaintLimit, cell_is_reachable, corridor_route, intent_cell_center, query_route,
        retain_build_footprints, route_exposure, world_to_intent_cell,
    },
    *,
};
use crate::nanobot::{InteractionRegion, cell_overlaps_circle, deposit_candidate_cells};
use bevy::prelude::Transform;
use std::collections::HashMap;

// Leave room for the home base and mandatory military support.
pub(super) const SUPPORT_PAINT_RESERVE: usize = 8;
const ECONOMY_EXPANSION_PAINT_LIMIT: usize = EXPANSION_PLAN_CELLS - SUPPORT_PAINT_RESERVE;
const ECONOMY_RETENTION_PAINT_LIMIT: usize = MAX_PLAN_CELLS - SUPPORT_PAINT_RESERVE;
const MINE_STARTUP_TICKS: u64 = 30 * crate::SIMULATION_HZ as u64;
// Both mines need an extractor and Source Stockpile builder, plus a shared Sink builder.
const PARALLEL_STARTUP_WORKERS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MineStartup {
    initial_amount: u32,
    deadline: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct OwnedDeposit {
    pub(super) id: u64,
    position: Vec2,
    haul_distance: f32,
    startup: Option<MineStartup>,
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

    fn claim(
        &mut self,
        mut mine: OwnedDeposit,
        amount: u32,
        tick: u64,
        budget: &mut WorkBudget,
    ) -> Option<()> {
        if self.owned_deposits.is_empty() {
            mine.startup = Some(MineStartup {
                initial_amount: amount,
                deadline: tick.saturating_add(MINE_STARTUP_TICKS),
            });
        }
        self.add(mine, budget).then_some(())
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
            Some((mut supported, _)) => {
                supported.startup = mine.startup.filter(|startup| {
                    deposit.amount >= startup.initial_amount && state.tick < startup.deadline
                });
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
            {
                economy.claim(mine, deposit.amount, state.tick, budget)?;
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

    economy.threatened = threatened_deposit(&economy.owned_deposits, owner, state, budget);
    if budget.exhausted {
        return None;
    }
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
    let (spare_workers, haul_overhead) =
        if !unchecked.is_empty() && economy.owned_deposits.len() > 1 {
            expansion_capacity(owner, state, &economy.owned_deposits, budget)?
        } else {
            (usize::MAX, 0.0)
        };
    if !unchecked.is_empty()
        && (spare_workers == 0
            || economy.owned_deposits.first().is_some_and(|mine| {
                mine.startup.is_some()
                    && !opening_can_staff_both_mines(owner, home, mine, state, budget)
            }))
    {
        economy.free_deposit_exists |= unchecked.into_iter().any(|deposit| {
            !budget.exhausted
                && !matches!(mine_support(owner, deposit, state, home, budget), Ok(None))
        });
        return (!budget.exhausted).then_some(economy);
    }

    let mut best: Option<(OwnedDeposit, &DepositState, f32)> = None;
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
        let could_claim = optimistic > best.as_ref().map_or(0.0, |(_, _, score)| *score);
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
        if score > best.as_ref().map_or(0.0, |(_, _, score)| *score)
            && (spare_workers > 1
                || source_support(owner, mine.position, SourceNeed::Space, state, budget)?
                    .has_space)
            && economy.can_fit(&mine, ECONOMY_EXPANSION_PAINT_LIMIT, budget)
        {
            best = Some((mine, deposit, score));
        }
    }
    if let Some((mine, deposit, _)) = best {
        let id = mine.id;
        economy.claim(mine, deposit.amount, state.tick, budget)?;
        economy.expansion = Some(id);
    }
    (!budget.exhausted).then_some(economy)
}

fn opening_can_staff_both_mines(
    owner: SwarmId,
    home: Vec2,
    mine: &OwnedDeposit,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> bool {
    let Some(support) = source_support(owner, mine.position, SourceNeed::Opening, state, budget)
    else {
        return false;
    };
    if support.stocked {
        return true;
    }
    let required_workers = PARALLEL_STARTUP_WORKERS - usize::from(support.has_space);

    let mut workers = Vec::new();
    for bot in state.bots {
        if !budget.spend(1) {
            return false;
        }
        if bot.owner == owner
            && bot.kind == NanobotType::Worker
            && bot.health > 0
            && bot.position.is_finite()
        {
            workers.push(bot.position);
        }
    }
    if workers.len() < required_workers {
        return false;
    }

    let mut reachable_workers = 0;
    for position in workers {
        let reachable = if state.route_query.is_some() {
            query_route(state, home, RouteGoal::Point(position), budget).is_some()
        } else {
            let Some(reachable) = reachable_position(owner, position, state, budget) else {
                return false;
            };
            reachable
        };
        if budget.exhausted {
            return false;
        }
        if reachable {
            reachable_workers += 1;
            if reachable_workers == required_workers {
                return true;
            }
        }
    }
    false
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

fn expansion_capacity(
    owner: SwarmId,
    state: &GameState<'_>,
    mines: &[OwnedDeposit],
    budget: &mut WorkBudget,
) -> Option<(usize, f32)> {
    let mut workers = 0usize;
    let mut haulers = 0usize;
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
    if haulers == 0 {
        return Some((0, 0.0));
    }

    // Reserve one Worker for shared upkeep, then keep an extractor and any
    // needed Source Stockpile builder at each existing mine.
    let mut assigned_workers = 1;
    let mut haul_distance = 0.0;
    for mine in mines {
        if !budget.spend(1) {
            return None;
        }
        assigned_workers += 1 + usize::from(
            !source_support(owner, mine.position, SourceNeed::Space, state, budget)?.has_space,
        );
        haul_distance += mine.haul_distance;
    }
    Some((
        workers.saturating_sub(assigned_workers),
        haul_distance / haulers as f32,
    ))
}

fn source_support(
    owner: SwarmId,
    position: Vec2,
    need: SourceNeed,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<SourceSupport> {
    state
        .source_query
        .map_or(Some(SourceSupport::default()), |query| {
            query(owner, position, need, &mut || budget.spend(1))
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
) -> Option<(u64, IVec2)> {
    if owned.is_empty() {
        return None;
    }
    if !budget.spend(owned.len() * 2) {
        return None;
    }
    let mut local_mines = HashMap::new();
    for mine in owned {
        for cell in [mine.gather_cell, mine.build_cell] {
            local_mines.entry(cell).or_insert(mine.id);
        }
    }
    let entities = state
        .bots
        .iter()
        .map(|bot| (bot.owner, bot.position, bot.health))
        .chain(state.structures.iter().map(|structure| {
            let health = if structure.kind == StructureKind::Planned {
                0
            } else {
                structure.health
            };
            (structure.owner, structure.position, health)
        }));
    for (other, position, health) in entities {
        if !budget.spend(1) {
            return None;
        }
        if other == owner || health == 0 {
            continue;
        }
        let cell = world_to_intent_cell(position);
        if let Some(&mine) = local_mines.get(&cell) {
            return Some((mine, cell));
        }
    }
    None
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
                startup: None,
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
