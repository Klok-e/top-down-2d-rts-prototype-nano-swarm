use super::{
    lifecycle::WorkBudget,
    materialize::{push_desired, world_to_intent_cell},
    *,
};

pub(super) fn affordable_defend_tiles(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    swarm: Option<&SwarmState>,
    state: &GameState<'_>,
    desired: &mut Vec<DesiredIntent>,
    paint_capacity: usize,
    budget: &mut WorkBudget,
) -> (Option<u64>, usize) {
    let charge_per_second = state.charge.drain_per_tick * crate::SIMULATION_HZ as f32;
    let upkeep =
        charge_per_second / state.charge.per_pulse * state.charge.material_per_pulse as f32;
    let baseline = tuning.strategy_defender_baseline as f32;
    let mineral_limit = if upkeep > 0.0 {
        let bank = swarm.map_or(0, |swarm| swarm.minerals);
        let reserve = tuning.strategy_mineral_reserve;
        let reserve_fraction = if reserve > 0 {
            (bank as f32 / reserve as f32).min(1.0)
        } else {
            1.0
        };
        let income = swarm.map_or(0.0, |swarm| swarm.mineral_income_per_second);
        let rate = income * tuning.strategy_income_spending_weight * reserve_fraction
            + bank.saturating_sub(reserve) as f32 / tuning.strategy_bank_spending_seconds;
        (rate / upkeep).clamp(baseline, EXPANSION_PLAN_CELLS.div_ceil(2) as f32)
    } else {
        EXPANSION_PLAN_CELLS.div_ceil(2) as f32
    };

    let mut living_defenders = 0;
    for bot in state.bots {
        if !budget.spend(1) {
            return (None, 1);
        }
        if bot.owner == owner && bot.kind == NanobotType::Defender && bot.health > 0 {
            living_defenders += 1;
        }
    }
    let charging_fraction = state.charge.drain_per_tick
        * f32::from(state.charge.pulse_interval_ticks)
        / state.charge.per_pulse;
    let rotation_slots = crate::nanobot::defender_rotation_capacity(living_defenders);
    let promised_slots = (mineral_limit * charging_fraction).ceil() as u32;
    let max_chargers = rotation_slots
        .max(promised_slots)
        .div_ceil(state.charge.max_defenders_per_charger)
        .max(1) as usize;
    let (key_charger, supplied_slots) = keep_charger_support(
        tuning,
        owner,
        state,
        max_chargers,
        desired,
        paint_capacity,
        budget,
    );
    let sustainable = if charging_fraction > 0.0 {
        mineral_limit
            .min(supplied_slots / charging_fraction)
            .max(baseline)
    } else {
        mineral_limit
    };
    // A pair of unique Swarm Tiles contributes one Defender to Population Demand.
    (key_charger, (sustainable as usize * 2).saturating_sub(1))
}

fn keep_charger_support(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    max_chargers: usize,
    desired: &mut Vec<DesiredIntent>,
    paint_capacity: usize,
    budget: &mut WorkBudget,
) -> (Option<u64>, f32) {
    let mut chargers = Vec::new();
    for structure in state.structures {
        if !budget.spend(1) {
            return (None, 0.0);
        }
        if structure.owner != owner
            || structure.kind != StructureKind::Charger
            || structure.health == 0
        {
            continue;
        }
        let cell = world_to_intent_cell(structure.position);
        if state
            .reachability
            .is_some_and(|map| !map.position_is_reachable(owner, structure.position, state.grid))
        {
            continue;
        }
        let supplied = structure.minerals.min(tuning.strategy_charger_reserve);
        let already_painted = state
            .grid
            .cell(cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner));
        chargers.push((
            std::cmp::Reverse((supplied, already_painted)),
            structure.id,
            cell,
            supplied,
        ));
    }
    chargers.sort_by_key(|&(priority, id, ..)| (priority, id));
    let mut key_charger = None;
    let mut kept = 0;
    let mut supplied = 0.0;
    for (_, id, cell, buffer) in chargers {
        let defend = DesiredIntent {
            cell,
            kind: IntentKind::Defend,
        };
        if !desired.contains(&defend)
            && (desired.len() >= paint_capacity
                || !push_desired(desired, owner, defend, state, true, budget))
        {
            continue;
        }
        key_charger.get_or_insert(id);
        supplied += buffer as f32 / tuning.strategy_charger_reserve as f32;
        if desired.len() < paint_capacity {
            push_desired(
                desired,
                owner,
                DesiredIntent {
                    cell,
                    kind: IntentKind::Corridor,
                },
                state,
                true,
                budget,
            );
        }
        kept += 1;
        if kept == max_chargers {
            break;
        }
    }
    (
        key_charger,
        supplied * state.charge.max_defenders_per_charger as f32,
    )
}

pub(super) fn add_production_room(
    tuning: &AutonomyTuning,
    swarm: &SwarmState,
    origins: impl Iterator<Item = IVec2>,
    state: &GameState<'_>,
    desired: &mut Vec<DesiredIntent>,
    paint_capacity: usize,
    budget: &mut WorkBudget,
) {
    if !swarm.production_saturated || swarm.minerals <= tuning.strategy_production_mineral_threshold
    {
        return;
    }
    let mut remaining = tuning.strategy_production_room_cells;
    let offsets = [IVec2::X, -IVec2::X, IVec2::Y, -IVec2::Y];
    for origin in origins {
        for offset in offsets {
            if remaining == 0 || desired.len() >= paint_capacity || budget.exhausted {
                return;
            }
            let intent = DesiredIntent {
                cell: origin + offset,
                kind: IntentKind::Build,
            };
            if push_desired(desired, swarm.id, intent, state, true, budget) {
                remaining -= 1;
            }
        }
    }
}
