use super::{
    lifecycle::WorkBudget,
    materialize::{
        CorridorPaintLimit, corridor_route, intent_cell_center, route_exposure,
        world_to_intent_cell,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MineStartup {
    initial_amount: u32,
    deadline: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct OwnedDeposit {
    pub(super) id: u64,
    position: Vec2,
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
        deposit: &DepositState,
        owner: SwarmId,
        state: &GameState<'_>,
        budget: &mut WorkBudget,
    ) {
        if self.owned_deposits.is_empty() {
            let mut has_source = false;
            for source in state.structures {
                if !budget.spend(1) {
                    return;
                }
                if source.owner == owner
                    && source.kind == StructureKind::Source
                    && source.health > 0
                    && source.minerals > 0
                    && world_to_intent_cell(source.position) == mine.gather_cell
                {
                    has_source = true;
                    break;
                }
            }
            if !has_source {
                mine.startup = Some(MineStartup {
                    initial_amount: deposit.amount,
                    deadline: state.tick.saturating_add(MINE_STARTUP_TICKS),
                });
            }
        }
        self.add(mine, budget);
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
    let mut economy = EconomyPlan::default();
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
            continue;
        }
        if gather_owners.contains(&owner) || deposit.owner == Some(owner) {
            if let Ok(Some((mine, _))) = mine_support(owner, deposit, state, home, budget)
                && economy.can_fit(&mine, ECONOMY_EXPANSION_PAINT_LIMIT, budget)
            {
                economy.claim(mine, deposit, owner, state, budget);
            }
            if budget.exhausted {
                return None;
            }
            continue;
        }
        if !gather_owners.is_empty() {
            economy
                .enemy_deposits
                .extend(gather_owners.into_iter().map(|other| (*deposit, other)));
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
                && state
                    .reachability
                    .is_none_or(|map| map.position_is_reachable(owner, bot.position, state.grid))
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
    if !economy.contested_deposits.is_empty() {
        economy.free_deposit_exists = true;
        return Some(economy);
    }

    if economy
        .owned_deposits
        .first()
        .is_some_and(|mine| mine.startup.is_some())
    {
        economy.free_deposit_exists |= unchecked.into_iter().any(|deposit| {
            !budget.exhausted
                && !matches!(mine_support(owner, deposit, state, home, budget), Ok(None))
        });
        return (!budget.exhausted).then_some(economy);
    }

    let mut best: Option<(OwnedDeposit, &DepositState, f32)> = None;
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
        let value = value - travel / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_travel_cost;
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
            && economy.can_fit(&mine, ECONOMY_EXPANSION_PAINT_LIMIT, budget)
        {
            best = Some((mine, deposit, score));
        }
    }
    if let Some((mine, deposit, _)) = best {
        economy.expansion = Some(mine.id);
        economy.claim(mine, deposit, owner, state, budget);
    }
    (!budget.exhausted).then_some(economy)
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
    let mut reachable = false;
    let mut gather_owners = BTreeSet::new();
    for cell in deposit_candidate_cells(state.grid, deposit.position, deposit.radius) {
        if !budget.spend(1) {
            return None;
        }
        if let Some(intent) = state.grid.cell(cell)
            && cell_overlaps_circle(cell, deposit.position, deposit.radius)
        {
            reachable |= state
                .reachability
                .is_none_or(|map| map.contains(owner, cell, state.grid));
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

fn deposit_region(deposit: &DepositState) -> InteractionRegion {
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
            || state.reachability.is_some_and(|map| {
                !map.contains(owner, gather_cell, state.grid)
                    || (state.route_query.is_none()
                        && !map.interaction_is_reachable(owner, region.within_cell(gather_cell)))
            })
        {
            continue;
        }
        let goal = RouteGoal::Interaction(region.within_cell(gather_cell));
        let (route, distance) = match corridor_route(state, budget, home, gather_cell, goal) {
            Ok(Some(route)) => route,
            Ok(None) if budget.exhausted => return Ok(None),
            Ok(None) => continue,
            Err(_) => {
                exceeds_paint_limit = true;
                continue;
            }
        };
        let viable = |cell| {
            state.grid.in_bounds(cell)
                && state
                    .reachability
                    .is_none_or(|map| map.contains(owner, cell, state.grid))
        };
        if !budget.spend(route.len() + 4) {
            return Ok(None);
        }
        if !route.iter().copied().all(viable) {
            continue;
        }

        let support = [IVec2::X, -IVec2::X, IVec2::Y, -IVec2::Y, IVec2::ZERO];
        let mut support = support.map(|offset| gather_cell + offset);
        support.sort_by(|a, b| {
            home.distance_squared(intent_cell_center(*a))
                .total_cmp(&home.distance_squared(intent_cell_center(*b)))
        });
        let Some(build_cell) = support.into_iter().find(|&cell| {
            if cell == gather_cell || !viable(cell) {
                return false;
            }
            state
                .terrain
                .iter()
                .all(|rock| budget.spend(1) && rock.admits_body(intent_cell_center(cell)))
        }) else {
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
