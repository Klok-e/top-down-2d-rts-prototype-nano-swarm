use super::{
    lifecycle::WorkBudget,
    staging::{DefendBlock, StagingFront},
    *,
};
use crate::{nanobot::InteractionRegion, navigation::Route};
use bevy::prelude::Transform;

pub(super) fn materialize_plan(
    candidate: PlanCandidate,
    mut staging: Option<StagingFront>,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> IntentPlan {
    let PlanCandidate {
        objective,
        economy,
        attack,
        anchor,
        attack_target: _,
        key_charger,
        defend_target,
        score: _,
    } = candidate;
    let anchor = staging.as_ref().map_or(anchor, StagingFront::point);
    let home = state
        .swarms
        .iter()
        .find(|swarm| swarm.id == owner)
        .map(|swarm| swarm.home)
        .unwrap_or(Vec2::ZERO);
    let home_cell = world_to_intent_cell(home);
    let mut desired = Vec::new();

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
    if let Some((deposit, deposit_cell)) = economy.primary {
        push_desired(
            &mut desired,
            owner,
            DesiredIntent {
                cell: deposit_cell,
                kind: IntentKind::Gather,
            },
            state,
            false,
            budget,
        );
        let destination = InteractionRegion::deposit(
            &Transform::from_translation(deposit.position.extend(0.0)),
            deposit.radius,
        )
        .within_cell(deposit_cell);
        paint_corridor(
            &mut desired,
            owner,
            state,
            budget,
            home,
            deposit_cell,
            RouteGoal::Interaction(destination),
        );
    }

    if let Some(charger) = key_charger {
        let charger_cell = world_to_intent_cell(charger.position);
        for kind in [IntentKind::Defend, IntentKind::Corridor] {
            push_desired(
                &mut desired,
                owner,
                DesiredIntent {
                    cell: charger_cell,
                    kind,
                },
                state,
                true,
                budget,
            );
        }
    }

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
    for cell in [anchor, anchor + IVec2::new(0, 1), anchor + IVec2::X] {
        if push_desired(
            &mut desired,
            owner,
            DesiredIntent {
                cell,
                kind: IntentKind::Build,
            },
            state,
            true,
            budget,
        ) {
            break;
        }
    }

    let block = DefendBlock::new(anchor, defend_target);
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

    if anchor != home_cell && economy.primary.is_none_or(|(_, cell)| cell != anchor) {
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
    desired.sort_by_key(|intent| (intent.cell.y, intent.cell.x, intent.kind.index()));

    IntentPlan {
        objective,
        primary_deposit: economy.primary.map(|(deposit, _)| deposit.id),
        attack,
        staging,
        key_charger: key_charger.map(|charger| charger.id),
        desired,
    }
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
    mut start: Vec2,
    target: IVec2,
    goal: RouteGoal,
) {
    let waypoints = if state.route_query.is_some() {
        let Some(route) = query_route(state, start, goal, budget) else {
            return;
        };
        route.waypoints
    } else {
        start = intent_cell_center(world_to_intent_cell(start));
        let end = intent_cell_center(target);
        for obstacle in state.terrain {
            if !budget.spend(1) || !obstacle.segment_clear(start, end) {
                return;
            }
        }
        vec![end]
    };
    let mut last = world_to_intent_cell(start);
    for end in waypoints {
        let span = (end - start).abs().max_element() / crate::ZONE_BLOCK_SIZE;
        let steps = (span.ceil() as usize).max(1);
        for step in 1..=steps {
            if !budget.spend(1) || desired.len() >= MAX_PLAN_CELLS {
                return;
            }
            let point = start.lerp(end, step as f32 / steps as f32);
            let cell = world_to_intent_cell(point);
            if cell != last {
                push_desired(
                    desired,
                    owner,
                    DesiredIntent {
                        cell,
                        kind: IntentKind::Corridor,
                    },
                    state,
                    false,
                    budget,
                );
                last = cell;
            }
        }
        start = end;
    }
}
