use super::{
    lifecycle::WorkBudget,
    materialize::{push_desired, world_to_intent_cell},
    *,
};
use crate::nanobot::{PlannedKind, footprint_intent_cells};

pub(super) struct DefenderBudget {
    baseline: f32,
    mineral_limit: f32,
    bank_fraction: f32,
    charging_fraction: f32,
    recovery_chargers: usize,
    charger_limit: usize,
}

impl DefenderBudget {
    pub(super) fn estimate(
        tuning: &AutonomyTuning,
        owner: SwarmId,
        swarm: Option<&SwarmState>,
        state: &GameState<'_>,
        budget: &mut WorkBudget,
    ) -> Self {
        let baseline = tuning.strategy_defender_baseline as f32;
        let bank = swarm.map_or(0, |swarm| swarm.minerals);
        let reserve = tuning.strategy_mineral_reserve;
        let bank_fraction = if reserve > 0 {
            (bank as f32 / reserve as f32).min(1.0)
        } else {
            1.0
        };

        let charge = state.charge;
        let charge_per_second = charge.drain_per_tick * crate::SIMULATION_HZ as f32;
        let upkeep = charge_per_second / charge.per_pulse * charge.material_per_pulse as f32;
        let charging_fraction =
            charge.drain_per_tick * f32::from(charge.pulse_interval_ticks) / charge.per_pulse;
        let upper_limit = EXPANSION_PLAN_CELLS.div_ceil(2) as f32;
        let mineral_limit = if upkeep > 0.0 {
            let income = swarm.map_or(0.0, |swarm| swarm.mineral_income_per_second);
            let rate = income * tuning.strategy_income_spending_weight
                + bank.saturating_sub(reserve) as f32 / tuning.strategy_bank_spending_seconds;
            (rate / upkeep).clamp(baseline, upper_limit)
        } else {
            upper_limit
        };

        let mut living_defenders = 0;
        for bot in state.bots {
            if !budget.spend(1) {
                break;
            }
            if bot.owner == owner && bot.kind == NanobotType::Defender && bot.health > 0 {
                living_defenders += 1;
            }
        }
        let recovery_chargers = crate::nanobot::defender_rotation_capacity(living_defenders)
            .div_ceil(charge.max_defenders_per_charger)
            .max(1) as usize;
        let funded = baseline + (mineral_limit - baseline) * bank_fraction;
        let growth_chargers = ((funded * charging_fraction).ceil() as u32)
            .div_ceil(charge.max_defenders_per_charger) as usize;
        Self {
            baseline,
            mineral_limit,
            bank_fraction,
            charging_fraction,
            recovery_chargers,
            charger_limit: growth_chargers.max(recovery_chargers),
        }
    }

    pub(super) fn tiles_with_supply(&self, supplied_slots: f32) -> usize {
        let sustainable = if self.charging_fraction > 0.0 {
            self.mineral_limit
                .min(supplied_slots / self.charging_fraction)
        } else {
            self.mineral_limit
        };
        let extra = (sustainable.floor() - self.baseline).max(0.0) * self.bank_fraction;
        // Use the smallest territory that yields this much rounded-up Population Demand.
        ((self.baseline + extra) as usize * 2).saturating_sub(1)
    }
}

pub(super) fn keep_charger_support(
    defense: &DefenderBudget,
    charger_reserve: u32,
    owner: SwarmId,
    state: &GameState<'_>,
    desired: &mut HashSet<DesiredIntent>,
    paint_capacity: usize,
    budget: &mut WorkBudget,
) -> (Option<u64>, f32) {
    let mut chargers = Vec::new();
    let half_extent = Vec2::splat(PlannedKind::Charger.footprint_half_extent());
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
        if let Some(map) = state.reachability {
            match map.position_is_reachable_with_budget(
                owner,
                structure.position,
                state.grid,
                &mut || budget.spend(1),
            ) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(()) => return (None, 0.0),
            }
        }
        let supplied = structure.minerals.min(charger_reserve);
        let footprint = footprint_intent_cells(structure.position, half_extent).collect::<Vec<_>>();
        let already_painted = footprint.iter().all(|&cell| {
            state
                .grid
                .cell(cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        });
        chargers.push((
            std::cmp::Reverse((supplied, already_painted)),
            structure.id,
            cell,
            supplied,
            footprint,
        ));
    }
    chargers.sort_by_key(|&(priority, id, ..)| (priority, id));
    let mut key_charger = None;
    let mut kept = 0;
    let mut supplied = 0.0;
    let mut kept_for_recovery = 0;
    for (_, id, cell, buffer, footprint) in chargers {
        let requires_recovery_paint = buffer < charger_reserve
            && !IntentKind::ALL
                .into_iter()
                .any(|kind| desired.contains(&DesiredIntent { cell, kind }));
        if requires_recovery_paint && kept_for_recovery >= defense.recovery_chargers {
            continue;
        }
        let mut added = Vec::new();
        let supported = footprint.into_iter().all(|cell| {
            let defend = DesiredIntent {
                cell,
                kind: IntentKind::Defend,
            };
            if desired.contains(&defend) {
                return true;
            }
            if desired.len() >= paint_capacity
                || !push_desired(desired, owner, defend, state, true, budget)
            {
                return false;
            }
            added.push(defend);
            true
        });
        if !supported {
            for defend in added {
                desired.remove(&defend);
            }
            continue;
        }
        key_charger.get_or_insert(id);
        supplied += buffer as f32 / charger_reserve as f32;
        kept_for_recovery += usize::from(requires_recovery_paint);
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
        if kept == defense.charger_limit {
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
    desired: &mut HashSet<DesiredIntent>,
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
