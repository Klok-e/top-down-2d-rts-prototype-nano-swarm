use super::{
    economy::EconomyPlan,
    lifecycle::WorkBudget,
    spending::{add_production_room, affordable_defend_tiles},
    staging::{DefendBlock, StagingFront},
    *,
};
use crate::navigation::Route;

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
        score: _,
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
    if anchor != home_cell {
        for cell in [anchor, anchor + IVec2::Y, anchor + IVec2::X] {
            let build = DesiredIntent {
                cell,
                kind: IntentKind::Build,
            };
            if desired.contains(&build)
                || push_desired(&mut desired, owner, build, state, true, budget)
            {
                break;
            }
        }
    }

    let minimum_cells = if staging.is_some() {
        super::staging::MINIMUM_STAGING_BLOCK_CELLS
    } else {
        1
    };
    let minimum = DefendBlock::new(anchor, minimum_cells);
    for cell in minimum.cells() {
        push_desired(
            &mut desired,
            owner,
            DesiredIntent {
                cell,
                kind: IntentKind::Defend,
            },
            state,
            true,
            budget,
        );
    }

    if anchor != home_cell && owned_deposits.iter().all(|mine| mine.gather_cell != anchor) {
        paint_corridor(
            &mut desired,
            owner,
            state,
            budget,
            home,
            anchor,
            RouteGoal::Point(intent_cell_center(anchor)),
        );
    }
    let (key_charger, affordable_tiles) =
        affordable_defend_tiles(tuning, owner, swarm, state, &mut desired, budget);
    if let Some(swarm) = swarm {
        add_production_room(
            tuning,
            swarm,
            home_cell,
            &owned_deposits,
            state,
            &mut desired,
            budget,
        );
    }

    let block = fit_defend_block(
        anchor,
        minimum,
        minimum_cells,
        affordable_tiles,
        &desired,
        budget,
    );
    if let Some(front) = staging.as_mut() {
        front.block = block;
    }
    for cell in block.cells() {
        push_desired(
            &mut desired,
            owner,
            DesiredIntent {
                cell,
                kind: IntentKind::Defend,
            },
            state,
            true,
            budget,
        );
    }
    desired.sort_by_key(|intent| (intent.cell.y, intent.cell.x, intent.kind.index()));

    IntentPlan {
        objective,
        owned_deposits,
        attack,
        staging,
        key_charger,
        desired,
    }
}

fn fit_defend_block(
    anchor: IVec2,
    minimum: DefendBlock,
    minimum_cells: usize,
    affordable_tiles: usize,
    desired: &[DesiredIntent],
    budget: &mut WorkBudget,
) -> DefendBlock {
    if affordable_tiles <= minimum_cells || !budget.spend(desired.len()) {
        return minimum;
    }
    let occupied_tiles: BTreeSet<_> = desired
        .iter()
        .map(|intent| (intent.cell.x, intent.cell.y))
        .collect();
    let extra_tiles = affordable_tiles.saturating_sub(occupied_tiles.len());
    let layer_room = MAX_PLAN_CELLS - desired.len();
    for count in (minimum_cells + 1..=affordable_tiles).rev() {
        let block = DefendBlock::new(anchor, count);
        if !minimum.cells().all(|cell| block.contains(cell)) {
            continue;
        }
        let mut new_tiles = 0;
        let mut new_layers = 0;
        for cell in block.cells() {
            if !budget.spend(1) {
                return minimum;
            }
            if !occupied_tiles.contains(&(cell.x, cell.y)) {
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

pub(super) fn push_desired(
    desired: &mut Vec<DesiredIntent>,
    owner: SwarmId,
    intent: DesiredIntent,
    state: &GameState<'_>,
    avoid_terrain: bool,
    budget: &mut WorkBudget,
) -> bool {
    if desired.len() >= MAX_PLAN_CELLS
        || !state.grid.in_bounds(intent.cell)
        || state
            .reachability
            .is_some_and(|reachability| !reachability.contains(owner, intent.cell, state.grid))
        || desired.contains(&intent)
        || !budget.spend(1)
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
    desired.push(intent);
    true
}

pub(super) fn intent_edits(
    owner: SwarmId,
    state: &GameState<'_>,
    previous: &[DesiredIntent],
    desired: &[DesiredIntent],
    cleanup_cursor: &mut Option<IVec2>,
    unreachable_cleanup_cursor: &mut Option<IVec2>,
    budget: &mut WorkBudget,
) -> Vec<IntentEdit> {
    let grid = state.grid;
    let mut edits = Vec::new();
    for intent in previous {
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
            if reachability.contains(owner, cell, grid) {
                last_completed = Some(cell);
                scanned += 1;
                continue;
            }
            for kind in IntentKind::ALL {
                if existing.has_owned(kind, owner) {
                    edits.push(IntentEdit {
                        cell,
                        kind,
                        action: IntentEditAction::Erase,
                    });
                }
            }
            last_completed = Some(cell);
            scanned += 1;
        }
        if scanned == MAX_ACTIVE_CELLS {
            *unreachable_cleanup_cursor = last_completed;
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
    desired: &mut Vec<DesiredIntent>,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
    start: Vec2,
    target: IVec2,
    goal: RouteGoal,
) {
    let Some((cells, _)) = corridor_route(state, budget, start, target, goal) else {
        return;
    };
    let previous_len = desired.len();
    for cell in cells {
        let intent = DesiredIntent {
            cell,
            kind: IntentKind::Corridor,
        };
        if !desired.contains(&intent) && !push_desired(desired, owner, intent, state, false, budget)
        {
            desired.truncate(previous_len);
            return;
        }
    }
}

pub(super) fn corridor_route(
    state: &GameState<'_>,
    budget: &mut WorkBudget,
    mut start: Vec2,
    target: IVec2,
    goal: RouteGoal,
) -> Option<(Vec<IVec2>, f32)> {
    let Route { waypoints, cost } = if state.route_query.is_some() {
        query_route(state, start, goal, budget)?
    } else {
        start = intent_cell_center(world_to_intent_cell(start));
        let end = intent_cell_center(target);
        for obstacle in state.terrain {
            if !budget.spend(1) || !obstacle.segment_clear(start, end) {
                return None;
            }
        }
        Route {
            waypoints: vec![end],
            cost: start.distance(end),
        }
    };
    let mut cells = vec![world_to_intent_cell(start)];
    for end in waypoints {
        let span = (end - start).abs().max_element() / crate::ZONE_BLOCK_SIZE;
        let steps = (span.ceil() as usize).max(1);
        for step in 1..=steps {
            if !budget.spend(1) {
                return None;
            }
            let point = start.lerp(end, step as f32 / steps as f32);
            let cell = world_to_intent_cell(point);
            if cells.contains(&cell) {
                continue;
            }
            if cells.len() >= MAX_PLAN_CELLS {
                return None;
            }
            cells.push(cell);
        }
        start = end;
    }
    Some((cells, cost))
}
