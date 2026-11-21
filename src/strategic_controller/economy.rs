use super::{
    lifecycle::WorkBudget,
    materialize::{corridor_route, intent_cell_center, route_exposure, world_to_intent_cell},
    *,
};
use crate::nanobot::{InteractionRegion, cell_overlaps_circle};
use bevy::prelude::Transform;

// Leave room for the home base and mandatory military support.
const ECONOMY_PAINT_LIMIT: usize = MAX_PLAN_CELLS - 8;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct OwnedDeposit {
    pub(super) id: u64,
    position: Vec2,
    pub(super) gather_cell: IVec2,
    build_cell: IVec2,
    pub(super) desired: Vec<DesiredIntent>,
}

#[derive(Default)]
pub(super) struct EconomyPlan {
    pub(super) owned_deposits: Vec<OwnedDeposit>,
    pub(super) desired: Vec<DesiredIntent>,
    pub(super) expansion: Option<u64>,
}

impl EconomyPlan {
    fn added_paint<'a>(
        &'a self,
        mine: &'a OwnedDeposit,
    ) -> impl Iterator<Item = DesiredIntent> + 'a {
        let desired = &self.desired;
        mine.desired
            .iter()
            .copied()
            .filter(move |intent| !desired.contains(intent))
    }

    fn can_fit(&self, mine: &OwnedDeposit) -> bool {
        self.desired.len() + self.added_paint(mine).count() <= ECONOMY_PAINT_LIMIT
    }

    fn add(&mut self, mine: OwnedDeposit) {
        let extra = self.added_paint(&mine).collect::<Vec<_>>();
        self.desired.extend(extra);
        self.owned_deposits.push(mine);
    }
}

fn available(deposit: &DepositState, owner: SwarmId) -> bool {
    deposit.amount > 0 && deposit.owner.is_none_or(|exclusive| exclusive == owner)
}

pub(super) fn validate_owned_deposits(
    owned: &[OwnedDeposit],
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<Option<u64>, ()> {
    for mine in owned {
        let valid = find_deposit(mine.id, state, budget)?
            .is_some_and(|deposit| available(deposit, owner) && deposit.position == mine.position);
        let support_reachable = state.reachability.is_none_or(|map| {
            mine.desired
                .iter()
                .all(|intent| map.contains(owner, intent.cell, state.grid))
        });
        if !valid || !support_reachable {
            return Ok(Some(mine.id));
        }
    }
    Ok(None)
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
        if let Some((supported, _)) = mine_support(owner, deposit, state, home, budget) {
            if !economy.can_fit(&supported) {
                return None;
            }
            economy.add(supported);
        } else if budget.exhausted {
            return None;
        }
    }

    let mut opportunities = Vec::new();

    for deposit in state.deposits.iter().take(MAX_DEPOSITS) {
        if !budget.spend(1) {
            return None;
        }
        if !available(deposit, owner) || previous.iter().any(|mine| mine.id == deposit.id) {
            continue;
        }
        let (own_gather, foreign_gather) = gather_paint(owner, deposit, state, budget)?;
        if own_gather || deposit.owner == Some(owner) {
            if let Some((mine, _)) = mine_support(owner, deposit, state, home, budget)
                && economy.can_fit(&mine)
            {
                economy.add(mine);
            }
            continue;
        }
        if foreign_gather {
            continue;
        }
        let terrain_exposure = route_exposure(home, deposit.position, state.terrain, budget);
        let enemy_exposure = enemy_home.map_or(0.0, |enemy| {
            let friendly_distance = home.distance(deposit.position);
            let enemy_distance = enemy.distance(deposit.position);
            friendly_distance / (friendly_distance + enemy_distance).max(crate::ZONE_BLOCK_SIZE)
        });
        let value = (deposit.amount as f32 + 1.0).ln() * tuning.strategy_expansion_income_weight
            - (terrain_exposure + enemy_exposure) * tuning.strategy_expansion_exposure_cost;
        opportunities.push((deposit, value));
    }
    if budget.exhausted {
        return None;
    }

    let threatened = threatened_deposit(&economy.owned_deposits, owner, state, budget);
    if budget.exhausted {
        return None;
    }
    if threatened.is_some() {
        return Some(economy);
    }

    let mut best: Option<(OwnedDeposit, f32)> = None;
    // Start with the cheapest plausible routes so distant candidates usually
    // need only an optimistic value bound, even when the map has many deposits.
    opportunities.sort_by(|(a, _), (b, _)| {
        home.distance_squared(a.position)
            .total_cmp(&home.distance_squared(b.position))
            .then_with(|| a.id.cmp(&b.id))
    });
    for (deposit, value) in opportunities {
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
        if optimistic <= best.as_ref().map_or(0.0, |(_, score)| *score) {
            continue;
        }
        let Some((mine, distance)) = mine_support(owner, deposit, state, home, budget) else {
            if budget.exhausted {
                return None;
            }
            continue;
        };
        let score = value - distance / crate::ZONE_BLOCK_SIZE * tuning.strategy_expansion_haul_cost;
        if economy.can_fit(&mine) && score > best.as_ref().map_or(0.0, |(_, score)| *score) {
            best = Some((mine, score));
        }
    }
    if let Some((mine, _)) = best {
        economy.expansion = Some(mine.id);
        economy.add(mine);
    }
    (!budget.exhausted).then_some(economy)
}

pub(super) fn threatened_deposit(
    owned: &[OwnedDeposit],
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<u64> {
    if owned.is_empty() {
        return None;
    }
    let entities = state
        .bots
        .iter()
        .take(MAX_BOTS)
        .map(|bot| (bot.owner, bot.position, bot.health))
        .chain(
            state
                .structures
                .iter()
                .take(MAX_STRUCTURES)
                .filter(|structure| structure.kind != StructureKind::Planned)
                .map(|structure| (structure.owner, structure.position, structure.health)),
        );
    for (other, position, health) in entities {
        if !budget.spend(1) {
            return None;
        }
        if other == owner || health == 0 {
            continue;
        }
        let cell = world_to_intent_cell(position);
        for mine in owned {
            if !budget.spend(1) {
                return None;
            }
            if cell == mine.gather_cell || cell == mine.build_cell {
                return Some(mine.id);
            }
        }
    }
    None
}

fn gather_paint(
    owner: SwarmId,
    deposit: &DepositState,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<(bool, bool)> {
    let mut own = false;
    let mut foreign = false;
    for cell in deposit_cells(deposit, state.grid) {
        if !budget.spend(1) {
            return None;
        }
        if let Some(intent) = state.grid.cell(cell)
            && cell_overlaps_circle(cell, deposit.position, deposit.radius)
        {
            own |= intent.has_owned(IntentKind::Gather, owner);
            foreign |= intent
                .owners(IntentKind::Gather)
                .any(|swarm| swarm != owner);
        }
    }
    Some((own, foreign))
}

fn deposit_cells(deposit: &DepositState, grid: &IntentGrid) -> impl Iterator<Item = IVec2> {
    let size = IVec2::new(grid.width(), grid.height());
    let map_min = -(size / 2);
    let radius = Vec2::splat(deposit.radius.max(0.0));
    let min = (((deposit.position - radius) / crate::ZONE_BLOCK_SIZE).ceil() - Vec2::ONE)
        .as_ivec2()
        .max(map_min);
    let max = world_to_intent_cell(deposit.position + radius).min(map_min + size - IVec2::ONE);
    (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| IVec2::new(x, y)))
}

fn mine_support(
    owner: SwarmId,
    deposit: &DepositState,
    state: &GameState<'_>,
    home: Vec2,
    budget: &mut WorkBudget,
) -> Option<(OwnedDeposit, f32)> {
    let center = world_to_intent_cell(deposit.position);
    let region = InteractionRegion::deposit(
        &Transform::from_translation(deposit.position.extend(0.0)),
        deposit.radius,
    );
    let cells = std::iter::once(center)
        .chain(deposit_cells(deposit, state.grid).filter(|cell| *cell != center));
    for gather_cell in cells {
        if !budget.spend(1) {
            return None;
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
        let Some((route, distance)) = corridor_route(state, budget, home, gather_cell, goal) else {
            if budget.exhausted {
                return None;
            }
            continue;
        };
        let viable = |cell| {
            state.grid.in_bounds(cell)
                && state
                    .reachability
                    .is_none_or(|map| map.contains(owner, cell, state.grid))
        };
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
        if !budget.spend(desired.len()) {
            return None;
        }
        return Some((
            OwnedDeposit {
                id: deposit.id,
                position: deposit.position,
                gather_cell,
                build_cell,
                desired,
            },
            distance,
        ));
    }
    None
}
