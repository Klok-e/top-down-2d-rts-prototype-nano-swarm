use super::economy::validate_owned_deposits;
use super::{
    lifecycle::WorkBudget,
    materialize::{route_exposure, world_to_intent_cell},
    *,
};

fn is_reachable(owner: SwarmId, state: &GameState<'_>, cell: IVec2) -> bool {
    state
        .reachability
        .is_none_or(|reachability| reachability.contains(owner, cell, state.grid))
}

fn is_target_reachable(owner: SwarmId, state: &GameState<'_>, position: Vec2) -> bool {
    state.reachability.is_none_or(|map| {
        map.contains(owner, world_to_intent_cell(position), state.grid)
            && map.position_is_reachable(owner, position, state.grid)
    })
}

pub(super) fn validate_plan(
    plan: &IntentPlan,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<PlanValidation, ()> {
    let validation = PlanValidation {
        lost_deposit: validate_owned_deposits(&plan.owned_deposits, owner, state, budget)?,
        ..PlanValidation::default()
    };
    if plan.desired.iter().any(|intent| {
        !is_reachable(owner, state, intent.cell)
            && !plan
                .owned_deposits
                .iter()
                .any(|mine| mine.desired.contains(intent))
    }) {
        return Ok(PlanValidation {
            invalidation: Some("existing Intent Plan left the swarm-reachable map".to_string()),
            ..validation
        });
    }
    if let Some(id) = plan.key_charger {
        let mut operational = false;
        for structure in state.structures {
            if !budget.spend(1) {
                return Err(());
            }
            if structure.id == id
                && structure.owner == owner
                && structure.kind == StructureKind::Charger
                && structure.health > 0
            {
                operational = true;
                break;
            }
        }
        if !operational {
            return Ok(PlanValidation {
                invalidation: Some(format!("key Charger {id} was lost")),
                ..validation
            });
        }
    }
    let Some(target) = plan.attack else {
        return Ok(validation);
    };
    let Some(observed) = observe_attack_target(target, owner, state, budget)? else {
        return Ok(PlanValidation {
            invalidation: Some(format!(
                "target {} {} disappeared or became unreachable",
                target.label(),
                target.id
            )),
            ..validation
        });
    };
    let moved = observed.cell != target.cell;
    let covered = plan
        .staging
        .as_ref()
        .is_some_and(|front| front.covers(observed.cell, owner, state));
    let left_destination = moved
        && plan
            .staging
            .as_ref()
            .is_some_and(|front| !front.destination_contains(observed.cell, owner, state));
    Ok(PlanValidation {
        invalidation: left_destination.then(|| {
            format!(
                "target {} {} moved outside its Defend block to {}",
                target.label(),
                target.id,
                observed.cell
            )
        }),
        attack_progressed: observed.health < target.health,
        covered_attack: covered,
        covered_attack_movement: moved && covered,
        ..validation
    })
}

impl AttackObligation {
    fn label(self) -> &'static str {
        match self.kind {
            AttackTargetKind::Structure => "structure",
            AttackTargetKind::Nanobot => "Nanobot",
        }
    }
}

pub(super) fn observe_attack_target(
    target: AttackObligation,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<Option<AttackObligation>, ()> {
    match target.kind {
        AttackTargetKind::Structure => {
            for structure in state.structures {
                if !budget.spend(1) {
                    return Err(());
                }
                if structure.id == target.id
                    && structure.owner != owner
                    && structure.health > 0
                    && structure.kind != StructureKind::Planned
                    && is_target_reachable(owner, state, structure.position)
                {
                    return Ok(Some(AttackObligation {
                        cell: world_to_intent_cell(structure.position),
                        health: structure.health,
                        ..target
                    }));
                }
            }
        }
        AttackTargetKind::Nanobot => {
            for bot in state.bots {
                if !budget.spend(1) {
                    return Err(());
                }
                if bot.id == target.id
                    && bot.owner != owner
                    && bot.health > 0
                    && is_target_reachable(owner, state, bot.position)
                {
                    return Ok(Some(AttackObligation {
                        cell: world_to_intent_cell(bot.position),
                        health: bot.health,
                        ..target
                    }));
                }
            }
        }
    }
    Ok(None)
}

pub(super) fn generate_candidates(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    expansion: Option<u64>,
    budget: &mut WorkBudget,
) -> Vec<PlanCandidate> {
    let owner_state = state.swarms.iter().find(|swarm| {
        let within_budget = budget.spend(1);
        within_budget && swarm.id == owner
    });
    let home = owner_state
        .map(|swarm| swarm.home)
        .or_else(|| {
            state
                .bots
                .iter()
                .take(MAX_BOTS)
                .find(|bot| bot.owner == owner)
                .map(|bot| bot.position)
        })
        .unwrap_or(Vec2::ZERO);
    let home_cell = world_to_intent_cell(home);
    let forces = estimate_forces(tuning, owner, state, budget);
    let supply = owner_state.map_or(0.0, |swarm| {
        swarm.minerals as f32 * tuning.strategy_ledger_supply_weight
    }) + state
        .structures
        .iter()
        .take(MAX_STRUCTURES)
        .filter(|structure| {
            structure.owner == owner
                && matches!(
                    structure.kind,
                    StructureKind::Source | StructureKind::Sink | StructureKind::Charger
                )
        })
        .map(|structure| structure.minerals as f32 * tuning.strategy_buffer_supply_weight)
        .sum::<f32>();
    let pressure_readiness_penalty = (tuning.strategy_readiness_supply_target - supply).max(0.0)
        * tuning.strategy_readiness_penalty_weight;

    let mut candidates = Vec::new();
    candidates.push(PlanCandidate {
        objective: expansion.map_or(Objective::HoldHome, Objective::SecureDeposit),
        attack: None,
        anchor: home_cell,
        attack_target: None,
        score: tuning.strategy_hold_base
            + supply.min(tuning.strategy_hold_supply_cap) * tuning.strategy_hold_supply_weight
            + forces.friendly_strength * tuning.strategy_hold_force_weight,
    });

    if expansion.is_some() {
        return candidates;
    }

    let enemy_home = state
        .swarms
        .iter()
        .filter(|swarm| swarm.id != owner)
        .min_by(|left, right| {
            home.distance_squared(left.home)
                .total_cmp(&home.distance_squared(right.home))
        })
        .map(|swarm| swarm.home);
    if let Some(enemy_home) = enemy_home {
        let target = world_to_intent_cell(enemy_home);
        if is_target_reachable(owner, state, enemy_home) {
            let distance = home.distance(enemy_home) / crate::ZONE_BLOCK_SIZE;
            let score = tuning.strategy_pressure_base
                + supply.min(tuning.strategy_pressure_supply_cap)
                    * tuning.strategy_pressure_supply_weight
                + forces.friendly_strength * tuning.strategy_pressure_force_weight
                - forces.enemy_strength * tuning.strategy_pressure_enemy_weight
                - forces.low_charge_defenders as f32 * tuning.strategy_pressure_low_charge_penalty
                - pressure_readiness_penalty
                - distance * tuning.strategy_pressure_distance_cost
                - route_exposure(home, enemy_home, state.terrain, budget)
                    * tuning.strategy_pressure_exposure_cost;
            candidates.push(PlanCandidate {
                objective: Objective::ForwardPressure(target),
                attack: None,
                anchor: home_cell,
                attack_target: Some(target),
                score,
            });
        }
    }

    for structure in state
        .structures
        .iter()
        .take(MAX_STRUCTURES)
        .filter(|structure| structure.owner != owner && structure.health > 0)
        .filter(|structure| {
            matches!(
                structure.kind,
                StructureKind::Facility
                    | StructureKind::Source
                    | StructureKind::Sink
                    | StructureKind::Charger
            )
        })
        .take(12)
    {
        if !budget.spend(1) {
            break;
        }
        let target = world_to_intent_cell(structure.position);
        if !is_target_reachable(owner, state, structure.position) {
            continue;
        }
        let strategic_value = match structure.kind {
            StructureKind::Charger => tuning.strategy_raid_charger_value,
            StructureKind::Source | StructureKind::Sink => tuning.strategy_raid_stockpile_value,
            StructureKind::Facility => tuning.strategy_raid_facility_value,
            StructureKind::Planned | StructureKind::Other => 0.0,
        };
        let distance = home.distance(structure.position) / crate::ZONE_BLOCK_SIZE;
        let vulnerability = health_deficit(structure.health, structure.max_health)
            * tuning.strategy_raid_vulnerability_weight;
        let score = tuning.strategy_raid_base
            + strategic_value
            + vulnerability
            + supply.min(tuning.strategy_raid_supply_cap) * tuning.strategy_raid_supply_weight
            + structure.minerals as f32 * tuning.strategy_raid_cargo_weight
            + forces.friendly_strength * tuning.strategy_raid_force_weight
            - forces.enemy_strength * tuning.strategy_raid_enemy_weight
            - pressure_readiness_penalty
            - distance * tuning.strategy_raid_distance_cost
            - route_exposure(home, structure.position, state.terrain, budget)
                * tuning.strategy_raid_exposure_cost;
        candidates.push(PlanCandidate {
            objective: Objective::RaidLogistics(structure.id),
            attack: Some(AttackObligation {
                kind: AttackTargetKind::Structure,
                id: structure.id,
                cell: target,
                health: structure.health,
            }),
            anchor: home_cell,
            attack_target: Some(target),
            score,
        });
    }

    let mut completed_structure_owners = BTreeSet::new();
    for structure in state.structures.iter().take(MAX_STRUCTURES) {
        if !budget.spend(1) {
            break;
        }
        if structure.health > 0
            && matches!(
                structure.kind,
                StructureKind::Facility
                    | StructureKind::Source
                    | StructureKind::Sink
                    | StructureKind::Charger
            )
        {
            completed_structure_owners.insert(structure.owner);
        }
    }
    for bot in state
        .bots
        .iter()
        .take(MAX_BOTS)
        .filter(|bot| bot.owner != owner && bot.health > 0)
        .take(12)
    {
        if !budget.spend(1) {
            break;
        }
        let target = world_to_intent_cell(bot.position);
        if !is_target_reachable(owner, state, bot.position) {
            continue;
        }
        let danger = match bot.kind {
            NanobotType::Defender => tuning.strategy_hunt_defender_value,
            NanobotType::Hauler => tuning.strategy_hunt_hauler_value,
            NanobotType::Worker => tuning.strategy_hunt_worker_value,
        };
        let distance = home.distance(bot.position) / crate::ZONE_BLOCK_SIZE;
        let vulnerability =
            health_deficit(bot.health, bot.max_health) * tuning.strategy_hunt_vulnerability_weight;
        let (cleanup_priority, cleanup_readiness_penalty) =
            if completed_structure_owners.contains(&bot.owner) {
                (
                    tuning.strategy_hunt_supported_base,
                    pressure_readiness_penalty,
                )
            } else {
                (tuning.strategy_hunt_remnant_base, 0.0)
            };
        let score = cleanup_priority
            + danger
            + vulnerability
            + forces.friendly_strength * tuning.strategy_hunt_force_weight
            - cleanup_readiness_penalty
            - distance * tuning.strategy_hunt_distance_cost
            - route_exposure(home, bot.position, state.terrain, budget)
                * tuning.strategy_hunt_exposure_cost;
        candidates.push(PlanCandidate {
            objective: Objective::HuntRemnant(bot.id),
            attack: Some(AttackObligation {
                kind: AttackTargetKind::Nanobot,
                id: bot.id,
                cell: target,
                health: bot.health,
            }),
            anchor: home_cell,
            attack_target: Some(target),
            score,
        });
    }
    candidates
}

pub(super) fn estimate_forces(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> ForceEstimate {
    let mut estimate = ForceEstimate::default();
    for bot in state.bots.iter().take(MAX_BOTS) {
        if !budget.spend(1) {
            break;
        }
        if bot.kind != NanobotType::Defender {
            continue;
        }
        let charge = if bot.charge.is_finite() {
            (bot.charge / bot.max_charge.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let strength = bot.health as f32
            * (tuning.strategy_force_base_weight + charge * tuning.strategy_force_charge_weight);
        if bot.owner == owner {
            estimate.friendly_defenders += 1;
            estimate.friendly_strength += strength;
            estimate.low_charge_defenders += usize::from(charge < tuning.min_front_charge);
        } else {
            estimate.enemy_strength += strength;
        }
    }
    estimate
}

fn health_deficit(health: u32, max_health: u32) -> f32 {
    100.0 * max_health.saturating_sub(health) as f32 / max_health.max(1) as f32
}
