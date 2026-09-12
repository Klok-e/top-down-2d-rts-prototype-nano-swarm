use super::{lifecycle::WorkBudget, *};

pub(super) fn materialize_plan(
    candidate: PlanCandidate,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> IntentPlan {
    let PlanCandidate {
        objective,
        economy,
        attack_targets,
        anchor,
        attack_target,
        key_charger,
        defend_target,
        score: _,
    } = candidate;
    let home = state
        .swarms
        .iter()
        .find(|swarm| swarm.id == owner)
        .map(|swarm| swarm.home)
        .unwrap_or(Vec2::ZERO);
    let home_cell = world_to_intent_cell(home);
    let mut desired = Vec::new();

    if let Some(deposit) = economy.primary {
        let deposit_cell = world_to_intent_cell(deposit.position);
        push_desired(
            &mut desired,
            DesiredIntent {
                cell: deposit_cell,
                kind: IntentKind::Gather,
            },
            state,
            false,
            budget,
        );
        for cell in line_cells(deposit_cell, home_cell, 32) {
            push_desired(
                &mut desired,
                DesiredIntent {
                    cell,
                    kind: IntentKind::Corridor,
                },
                state,
                true,
                budget,
            );
        }
    }

    if let Some(charger) = key_charger {
        let charger_cell = world_to_intent_cell(charger.position);
        for kind in [IntentKind::Defend, IntentKind::Corridor] {
            push_desired(
                &mut desired,
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

    let mut defend_candidates = vec![home_cell, anchor];
    let mut attack_cells = attack_targets
        .iter()
        .map(|target| target.cell)
        .collect::<Vec<_>>();
    if attack_cells.is_empty()
        && let Some(target) = attack_target
    {
        attack_cells.push(target);
    }
    defend_candidates.extend(attack_cells.iter().copied());
    for target in attack_cells {
        for radius in 1_i32..=2 {
            for y in -radius..=radius {
                for x in -radius..=radius {
                    if x.abs().max(y.abs()) == radius {
                        defend_candidates.push(target + IVec2::new(x, y));
                    }
                }
            }
        }
    }
    for radius in 0_i32..=3 {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x.abs().max(y.abs()) == radius {
                    defend_candidates.push(anchor + IVec2::new(x, y));
                }
            }
        }
    }
    for cell in defend_candidates {
        if desired
            .iter()
            .filter(|intent| intent.kind == IntentKind::Defend)
            .count()
            >= defend_target
        {
            break;
        }
        push_desired(
            &mut desired,
            DesiredIntent {
                cell,
                kind: IntentKind::Defend,
            },
            state,
            true,
            budget,
        );
    }

    for cell in line_cells(home_cell, anchor, 32) {
        push_desired(
            &mut desired,
            DesiredIntent {
                cell,
                kind: IntentKind::Corridor,
            },
            state,
            true,
            budget,
        );
    }
    desired.sort_by_key(|intent| (intent.cell.y, intent.cell.x, intent.kind.index()));

    IntentPlan {
        objective,
        primary_deposit: economy.primary.map(|deposit| deposit.id),
        attack_targets,
        key_charger: key_charger.map(|charger| charger.id),
        desired,
    }
}

pub(super) fn push_desired(
    desired: &mut Vec<DesiredIntent>,
    intent: DesiredIntent,
    state: &GameState<'_>,
    avoid_terrain: bool,
    budget: &mut WorkBudget,
) -> bool {
    if desired.len() >= MAX_PLAN_CELLS
        || !state.grid.in_bounds(intent.cell)
        || desired.contains(&intent)
        || !budget.spend(1)
    {
        return false;
    }
    if avoid_terrain {
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
    grid: &IntentGrid,
    desired: &[DesiredIntent],
    cleanup_cursor: &mut Option<IVec2>,
    budget: &mut WorkBudget,
) -> Vec<IntentEdit> {
    let mut edits = Vec::new();
    for intent in desired {
        if !budget.spend(1) {
            break;
        }
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
    edits.sort_by_key(|edit| {
        (
            edit.action == IntentEditAction::Erase,
            edit.cell.y,
            edit.cell.x,
            edit.kind.index(),
        )
    });
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

pub(super) fn midpoint_cell(left: IVec2, right: IVec2) -> IVec2 {
    IVec2::new((left.x + right.x) / 2, (left.y + right.y) / 2)
}

pub(super) fn line_cells(start: IVec2, target: IVec2, limit: usize) -> Vec<IVec2> {
    let mut cells = Vec::new();
    let mut current = start;
    cells.push(current);
    while current != target && cells.len() < limit {
        current += (target - current).signum();
        cells.push(current);
    }
    cells
}
