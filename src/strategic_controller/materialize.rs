use super::{
    economy::{EconomyPlan, SUPPORT_PAINT_RESERVE},
    lifecycle::WorkBudget,
    spending::{DefenderBudget, keep_charger_support},
    staging::{DefendBlock, StagingFront},
    *,
};
use crate::{nanobot::footprint_intent_cells, navigation::Route};
use std::collections::VecDeque;

pub(super) fn materialize_plan(
    candidate: PlanCandidate,
    economy: EconomyPlan,
    mut staging: Option<StagingFront>,
    owner: SwarmId,
    state: &GameState<'_>,
    tuning: &AutonomyTuning,
    budget: &mut WorkBudget,
) -> IntentPlan {
    let PlanCandidate {
        objective,
        attack,
        anchor,
        attack_target: _,
        approach: _,
        score: _,
        protected_home,
    } = candidate;
    let anchor = staging.as_ref().map_or(anchor, StagingFront::point);
    let swarm = state.swarms.iter().find(|swarm| swarm.id == owner);
    let home = swarm.map_or(Vec2::ZERO, |swarm| swarm.home);
    let home_cell = world_to_intent_cell(home);
    let EconomyPlan {
        owned_deposits,
        mut desired,
        ..
    } = economy;

    push_desired(
        &mut desired,
        owner,
        DesiredIntent {
            cell: home_cell,
            kind: IntentKind::Corridor,
        },
        state,
        true,
        budget,
    );
    push_desired(
        &mut desired,
        owner,
        DesiredIntent {
            cell: home_cell,
            kind: IntentKind::Build,
        },
        state,
        true,
        budget,
    );
    keep_spare_build_room(
        owner,
        tuning.strategy_production_room_cells,
        std::iter::once(home_cell).chain(owned_deposits.iter().map(|mine| mine.build_cell)),
        state,
        &mut desired,
        budget,
    );
    let paint_capacity = EXPANSION_PLAN_CELLS
        .max(desired.len() + SUPPORT_PAINT_RESERVE)
        .min(MAX_PLAN_CELLS);

    let defense = DefenderBudget::estimate(tuning, owner, swarm, state, budget);
    let (key_charger, supplied_slots) = keep_charger_support(
        &defense,
        tuning.strategy_charger_reserve,
        owner,
        state,
        &mut desired,
        paint_capacity.saturating_sub(1),
        budget,
    );
    let minimum_cells = if staging.is_some() {
        super::staging::MINIMUM_STAGING_BLOCK_CELLS
    } else {
        1
    }
    .min(paint_capacity.saturating_sub(desired.len()).max(1));
    let block_at_anchor = |count| DefendBlock::new(anchor, count).avoiding(anchor, protected_home);
    let minimum = block_at_anchor(minimum_cells);
    let anchor_ground = state.reachability.and_then(|map| {
        map.planning_point_with_budget(owner, anchor, state.grid, &mut || budget.spend(1))
            .ok()
            .flatten()
            .map(|point| (anchor, point))
    });
    paint_defend_block(minimum, anchor_ground, owner, state, &mut desired, budget);

    if anchor != home_cell && owned_deposits.iter().all(|mine| mine.gather_cell != anchor) {
        paint_corridor(
            &mut desired,
            owner,
            state,
            budget,
            home,
            anchor,
            protected_home,
        );
    }

    let block = fit_defend_block(
        block_at_anchor,
        minimum,
        minimum_cells,
        defense.tiles_with_supply(supplied_slots),
        &desired,
        paint_capacity,
        budget,
    );
    if let Some(front) = staging.as_mut() {
        front.block = block;
    }
    if block != minimum {
        paint_defend_block(block, anchor_ground, owner, state, &mut desired, budget);
    }
    IntentPlan {
        objective,
        owned_deposits,
        attack,
        staging,
        key_charger,
        desired,
        reachability: state.reachability.cloned(),
        grid_size: IVec2::new(state.grid.width(), state.grid.height()),
    }
}

pub(super) fn retain_build_footprints(
    owner: SwarmId,
    state: &GameState<'_>,
    paint_limit: usize,
    budget: &mut WorkBudget,
) -> Option<HashSet<DesiredIntent>> {
    let mut desired = HashSet::new();
    let mut required = Vec::new();
    for footprint in state.build_footprints {
        if !budget.spend(1) {
            return None;
        }
        if footprint.owner != owner {
            continue;
        }
        if let Some(map) = state.reachability
            && !map
                .position_is_reachable_with_budget(owner, footprint.center, state.grid, &mut || {
                    budget.spend(1)
                })
                .ok()?
        {
            continue;
        }
        required.clear();
        let mut accessible = true;
        for cell in footprint_intent_cells(footprint.center, footprint.half) {
            if !budget.spend(1) {
                return None;
            }
            if !cell_is_reachable(owner, state, cell, budget)? {
                accessible = false;
                break;
            }
            required.push(DesiredIntent {
                cell,
                kind: IntentKind::Build,
            });
        }
        if accessible {
            desired.extend(required.iter().copied());
            if desired.len() > paint_limit {
                return None;
            }
        }
    }
    Some(desired)
}

fn keep_spare_build_room(
    owner: SwarmId,
    allowance: u32,
    origins: impl Iterator<Item = IVec2>,
    state: &GameState<'_>,
    desired: &mut HashSet<DesiredIntent>,
    budget: &mut WorkBudget,
) {
    const MAX_SPARE_CELL_WORK: usize = 512;

    for origin in origins {
        let mut remaining = allowance;
        for offset in [IVec2::X, -IVec2::X, IVec2::Y, -IVec2::Y] {
            if remaining == 0 {
                break;
            }
            if desired.len() >= MAX_PLAN_CELLS - SUPPORT_PAINT_RESERVE || budget.exhausted {
                return;
            }
            let cell = origin + offset;
            if desired.contains(&DesiredIntent {
                cell,
                kind: IntentKind::Gather,
            }) {
                continue;
            }
            if budget.optional_probe(MAX_SPARE_CELL_WORK, |budget| {
                push_desired(
                    desired,
                    owner,
                    DesiredIntent {
                        cell,
                        kind: IntentKind::Build,
                    },
                    state,
                    true,
                    budget,
                )
            }) {
                remaining -= 1;
            }
        }
    }
}

fn paint_defend_block(
    block: DefendBlock,
    anchor_ground: Option<(IVec2, Vec2)>,
    owner: SwarmId,
    state: &GameState<'_>,
    desired: &mut HashSet<DesiredIntent>,
    budget: &mut WorkBudget,
) {
    let mut paint = |cell, budget: &mut WorkBudget| {
        push_desired(
            desired,
            owner,
            DesiredIntent {
                cell,
                kind: IntentKind::Defend,
            },
            state,
            true,
            budget,
        );
    };
    let (Some(map), Some((anchor, ground))) = (state.reachability, anchor_ground) else {
        for cell in block.cells() {
            paint(cell, budget);
        }
        return;
    };
    let mut reached = HashSet::from([anchor]);
    let mut frontier = VecDeque::from([(anchor, ground)]);
    while let Some((cell, ground)) = frontier.pop_front() {
        paint(cell, budget);
        for offset in [IVec2::X, -IVec2::X, IVec2::Y, -IVec2::Y] {
            let next = cell + offset;
            if !block.contains(next) || reached.contains(&next) {
                continue;
            }
            match map.visible_ground_from(owner, ground, next, state.grid, &mut || budget.spend(1))
            {
                Ok(Some(point)) => {
                    reached.insert(next);
                    frontier.push_back((next, point));
                }
                Ok(None) => {}
                Err(()) => return,
            }
        }
    }
}

fn fit_defend_block(
    block_at_anchor: impl Fn(usize) -> DefendBlock,
    minimum: DefendBlock,
    minimum_cells: usize,
    affordable_tiles: usize,
    desired: &HashSet<DesiredIntent>,
    paint_capacity: usize,
    budget: &mut WorkBudget,
) -> DefendBlock {
    if affordable_tiles <= minimum_cells || !budget.spend(desired.len()) {
        return minimum;
    }
    let occupied_tiles: HashSet<_> = desired.iter().map(|intent| intent.cell).collect();
    let extra_tiles = affordable_tiles.saturating_sub(occupied_tiles.len());
    let layer_room = paint_capacity.saturating_sub(desired.len());
    for count in (minimum_cells + 1..=affordable_tiles).rev() {
        let block = block_at_anchor(count);
        if !minimum.cells().all(|cell| block.contains(cell)) {
            continue;
        }
        let mut new_tiles = 0;
        let mut new_layers = 0;
        for cell in block.cells() {
            if !budget.spend(1) {
                return minimum;
            }
            if !occupied_tiles.contains(&cell) {
                new_tiles += 1;
            }
            if !desired.contains(&DesiredIntent {
                cell,
                kind: IntentKind::Defend,
            }) {
                new_layers += 1;
            }
            if new_tiles > extra_tiles || new_layers > layer_room {
                break;
            }
        }
        if new_tiles <= extra_tiles && new_layers <= layer_room {
            return block;
        }
    }
    minimum
}

pub(super) fn cell_is_reachable(
    owner: SwarmId,
    state: &GameState<'_>,
    cell: IVec2,
    budget: &mut WorkBudget,
) -> Option<bool> {
    if !state.grid.in_bounds(cell) {
        return Some(false);
    }
    state.reachability.map_or(Some(true), |map| {
        map.contains_with_budget(owner, cell, state.grid, &mut || budget.spend(1))
            .ok()
    })
}

pub(super) fn push_desired(
    desired: &mut HashSet<DesiredIntent>,
    owner: SwarmId,
    intent: DesiredIntent,
    state: &GameState<'_>,
    avoid_terrain: bool,
    budget: &mut WorkBudget,
) -> bool {
    if !budget.spend(1)
        || desired.len() >= MAX_PLAN_CELLS
        || desired.contains(&intent)
        || cell_is_reachable(owner, state, intent.cell, budget) != Some(true)
    {
        return false;
    }
    if avoid_terrain && state.reachability.is_none() {
        let center = intent_cell_center(intent.cell);
        for obstacle in state.terrain {
            if !budget.spend(1) {
                return false;
            }
            if !obstacle.admits_body(center) {
                return false;
            }
        }
    }
    desired.insert(intent)
}

pub(super) fn intent_edits(
    owner: SwarmId,
    state: &GameState<'_>,
    previous: Option<&HashSet<DesiredIntent>>,
    desired: &HashSet<DesiredIntent>,
    cleanup_cursor: &mut Option<IVec2>,
    unreachable_cleanup_cursor: &mut Option<IVec2>,
    budget: &mut WorkBudget,
) -> Vec<IntentEdit> {
    let grid = state.grid;
    let mut edits = Vec::new();
    for intent in previous.into_iter().flatten() {
        if !desired.contains(intent)
            && grid
                .cell(intent.cell)
                .is_some_and(|cell| cell.has_owned(intent.kind, owner))
        {
            edits.push(IntentEdit {
                cell: intent.cell,
                kind: intent.kind,
                action: IntentEditAction::Erase,
            });
        }
    }
    for intent in desired {
        if !grid
            .cell(intent.cell)
            .is_some_and(|cell| cell.has_owned(intent.kind, owner))
        {
            edits.push(IntentEdit {
                cell: intent.cell,
                kind: intent.kind,
                action: IntentEditAction::Paint,
            });
        }
    }
    let previous_cursor = *cleanup_cursor;
    let mut last_completed = None;
    let mut active_cells = grid.iter_active_cells_after(previous_cursor).peekable();
    let mut scanned = 0;
    'cleanup: while scanned < MAX_ACTIVE_CELLS {
        let Some((cell, existing)) = active_cells.next() else {
            *cleanup_cursor = None;
            break;
        };
        for kind in IntentKind::ALL {
            if !budget.spend(1) {
                *cleanup_cursor = last_completed.or(previous_cursor);
                break 'cleanup;
            }
            if existing.has_owned(kind, owner) && !desired.contains(&DesiredIntent { cell, kind }) {
                edits.push(IntentEdit {
                    cell,
                    kind,
                    action: IntentEditAction::Erase,
                });
            }
        }
        last_completed = Some(cell);
        scanned += 1;
        if active_cells.peek().is_none() {
            *cleanup_cursor = None;
            break;
        }
        *cleanup_cursor = last_completed;
    }
    if let Some(reachability) = state.reachability {
        let previous_cursor = *unreachable_cleanup_cursor;
        let mut last_completed = None;
        let mut active_cells = grid.iter_active_cells_after(previous_cursor).peekable();
        let mut scanned = 0;
        while scanned < MAX_ACTIVE_CELLS {
            let Some((cell, existing)) = active_cells.next() else {
                *unreachable_cleanup_cursor = None;
                break;
            };
            if !budget.spend(1) {
                *unreachable_cleanup_cursor = last_completed.or(previous_cursor);
                break;
            }
            if IntentKind::ALL
                .into_iter()
                .any(|kind| existing.has_owned(kind, owner))
            {
                let Ok(reachable) =
                    reachability.contains_with_budget(owner, cell, grid, &mut || budget.spend(1))
                else {
                    *unreachable_cleanup_cursor = last_completed.or(previous_cursor);
                    break;
                };
                if !reachable {
                    for kind in IntentKind::ALL {
                        if existing.has_owned(kind, owner) {
                            edits.push(IntentEdit {
                                cell,
                                kind,
                                action: IntentEditAction::Erase,
                            });
                        }
                    }
                }
            }
            last_completed = Some(cell);
            scanned += 1;
        }
        if scanned == MAX_ACTIVE_CELLS {
            *unreachable_cleanup_cursor = last_completed;
        }
    }
    edits.sort_by_key(|edit| {
        (
            edit.action == IntentEditAction::Erase,
            edit.cell.y,
            edit.cell.x,
            edit.kind.index(),
        )
    });
    edits.dedup();
    edits
}

pub(super) fn route_exposure(
    start: Vec2,
    end: Vec2,
    terrain: &[Obstacle],
    budget: &mut WorkBudget,
) -> f32 {
    let mut exposed = 0.0;
    let mut samples = 0.0;
    for step in 1..=6 {
        let position = start.lerp(end, step as f32 / 7.0);
        let mut sample_is_exposed = false;
        for obstacle in terrain {
            if !budget.spend(1) {
                // Incomplete terrain inspection is never evidence of a clear route.
                return 1.0;
            }
            if obstacle.surface_distance(position) < crate::navigation::BODY_RADIUS * 2.0 {
                sample_is_exposed = true;
                break;
            }
        }
        samples += 1.0;
        exposed += if sample_is_exposed { 1.0 } else { 0.0 };
    }
    exposed / samples
}

pub(super) fn world_to_intent_cell(position: Vec2) -> IVec2 {
    (position / crate::ZONE_BLOCK_SIZE).floor().as_ivec2()
}

pub(super) fn intent_cell_center(cell: IVec2) -> Vec2 {
    (cell.as_vec2() + Vec2::splat(0.5)) * crate::ZONE_BLOCK_SIZE
}

pub(super) fn query_route(
    state: &GameState<'_>,
    start: Vec2,
    goal: RouteGoal,
    budget: &mut WorkBudget,
) -> Option<Route> {
    let route_query = state.route_query?;
    if !budget.spend(1) {
        return None;
    }
    match route_query(start, goal, &mut || budget.spend(1))? {
        RouteOutcome::Found(route) => Some(route),
        RouteOutcome::Unreachable => None,
    }
}

fn paint_corridor(
    desired: &mut HashSet<DesiredIntent>,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
    start: Vec2,
    target: IVec2,
    protected_home: Option<IVec2>,
) {
    let goal = RouteGoal::Point(intent_cell_center(target));
    let Ok(Some((cells, _))) = corridor_route(owner, state, budget, start, target, goal) else {
        return;
    };
    if !budget.spend(cells.len()) || protected_home.is_some_and(|home| cells.contains(&home)) {
        return;
    }
    let mut added = Vec::new();
    for cell in cells {
        let intent = DesiredIntent {
            cell,
            kind: IntentKind::Corridor,
        };
        if desired.contains(&intent) {
            continue;
        }
        if desired.len() >= EXPANSION_PLAN_CELLS
            || !push_desired(desired, owner, intent, state, false, budget)
        {
            for intent in added {
                desired.remove(&intent);
            }
            return;
        }
        added.push(intent);
    }
}

#[derive(Debug)]
pub(super) struct CorridorPaintLimit;

pub(super) fn corridor_route(
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
    mut start: Vec2,
    target: IVec2,
    goal: RouteGoal,
) -> Result<Option<(Vec<IVec2>, f32)>, CorridorPaintLimit> {
    let Route { waypoints, cost } = if state.route_query.is_some() {
        let Some(route) = query_route(state, start, goal, budget) else {
            return Ok(None);
        };
        route
    } else {
        start = intent_cell_center(world_to_intent_cell(start));
        let end = intent_cell_center(target);
        for obstacle in state.terrain {
            if !budget.spend(1) || !obstacle.segment_clear(start, end) {
                return Ok(None);
            }
        }
        Route {
            waypoints: vec![end],
            cost: start.distance(end),
        }
    };
    let origin = start;
    let mut samples = state.reachability.map(|_| vec![start]);
    let mut cells = vec![world_to_intent_cell(start)];
    let mut visited = HashSet::from([cells[0]]);
    for &end in &waypoints {
        let span = (end - start).abs().max_element() / crate::ZONE_BLOCK_SIZE;
        let steps = (span.ceil() as usize).max(1);
        for step in 1..=steps {
            if !budget.spend(1) {
                return Ok(None);
            }
            let point = start.lerp(end, step as f32 / steps as f32);
            let cell = world_to_intent_cell(point);
            if !visited.insert(cell) {
                continue;
            }
            if cells.len() >= MAX_PLAN_CELLS {
                return Err(CorridorPaintLimit);
            }
            cells.push(cell);
            if let Some(samples) = &mut samples {
                samples.push(point);
            }
        }
        start = end;
    }
    if let Some(map) = state.reachability
        && let Some(samples) = samples
        && map
            .remember_route_ground(owner, origin, &waypoints, &samples, &mut || budget.spend(1))
            .is_err()
    {
        return Ok(None);
    }
    Ok(Some((cells, cost)))
}
