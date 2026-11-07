use super::{
    lifecycle::WorkBudget,
    materialize::{intent_cell_center, midpoint_cell, route_exposure, world_to_intent_cell},
    *,
};

fn is_reachable(owner: SwarmId, state: &GameState<'_>, cell: IVec2) -> bool {
    state
        .reachability
        .is_none_or(|reachability| reachability.contains(owner, cell, state.grid))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn coordinate_attack_targets(
    tuning: &AutonomyTuning,
    selected: &mut PlanCandidate,
    candidates: &[PlanCandidate],
    current: Option<&IntentPlan>,
    owner: SwarmId,
    state: &GameState<'_>,
    reviewed_supportable_fronts: Option<usize>,
    budget: &mut WorkBudget,
) -> Option<String> {
    if selected.attack_targets.is_empty() {
        return None;
    }

    let mut progressing = Vec::new();
    if let Some(current) = current {
        for previous in &current.attack_targets {
            let observed = match observe_attack_target(*previous, owner, state, budget) {
                Ok(Some(observed)) => observed,
                Ok(None) | Err(()) => continue,
            };
            if observed.health < previous.health {
                progressing.push(observed);
            }
        }
    }

    if reviewed_supportable_fronts
        .unwrap_or_else(|| supportable_attack_fronts(tuning, owner, state, budget))
        < 2
    {
        if let Some(incumbent) = progressing.first().copied()
            && !selected
                .attack_targets
                .iter()
                .any(|target| target.kind == incumbent.kind && target.id == incumbent.id)
            && let Some(refreshed) = candidates.iter().find(|candidate| {
                candidate
                    .attack_targets
                    .iter()
                    .any(|target| target.kind == incumbent.kind && target.id == incumbent.id)
            })
        {
            *selected = refreshed.clone();
            selected.attack_targets = vec![incumbent];
            return Some(format!(
                "retained progressing attack at {}; one front is supportable",
                incumbent.cell
            ));
        } else {
            selected.attack_targets.truncate(1);
        }
        return Some("limited attack to one supportable front".to_string());
    }

    let selected_primary = selected.attack_targets[0];
    let mut coordinated = Vec::with_capacity(2);
    let retained_incumbent = progressing
        .into_iter()
        .find(|target| target.kind != selected_primary.kind || target.id != selected_primary.id);
    if let Some(incumbent) = retained_incumbent {
        coordinated.push(incumbent);
    }
    coordinated.push(selected_primary);

    if coordinated.len() < 2 {
        let secondary = candidates
            .iter()
            .filter_map(|candidate| {
                if !budget.spend(1) {
                    return None;
                }
                candidate
                    .attack_targets
                    .first()
                    .copied()
                    .map(|target| (candidate.score, target))
            })
            .filter(|(_, target)| {
                !coordinated
                    .iter()
                    .any(|existing| existing.kind == target.kind && existing.id == target.id)
            })
            .max_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, target)| target);
        if let Some(secondary) = secondary {
            coordinated.push(secondary);
        }
    }
    selected.attack_targets = coordinated;
    if let Some(incumbent) = retained_incumbent
        && selected.attack_targets.len() == 2
    {
        Some(format!(
            "retained progressing attack at {} while opening a supported second front",
            incumbent.cell
        ))
    } else if selected.attack_targets.len() == 2 {
        Some("opened two supportable attack fronts".to_string())
    } else {
        Some("selected one live attack target".to_string())
    }
}

pub(super) fn supportable_attack_fronts(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> usize {
    let mut ready_defenders = 0;
    for bot in state.bots.iter().take(MAX_BOTS) {
        if !budget.spend(1) {
            break;
        }
        if bot.owner == owner
            && bot.kind == NanobotType::Defender
            && bot.health as f32 >= bot.max_health as f32 * tuning.min_front_health_fraction
            && bot.charge.is_finite()
            && bot.charge >= bot.max_charge * tuning.min_front_charge
        {
            ready_defenders += 1;
        }
    }
    if ready_defenders < tuning.min_second_front_defenders as usize {
        return 1;
    }
    let supplied_charger = state
        .structures
        .iter()
        .take(MAX_STRUCTURES)
        .any(|structure| {
            budget.spend(1)
                && structure.owner == owner
                && structure.kind == StructureKind::Charger
                && structure.health > 0
                && structure.minerals > 0
        });
    if supplied_charger { 2 } else { 1 }
}

pub(super) fn adapt_pressure_anchor(
    tuning: &AutonomyTuning,
    candidate: &mut PlanCandidate,
    current: Option<&IntentPlan>,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) {
    let Some(target) = candidate.attack_target else {
        return;
    };
    let home = state
        .swarms
        .iter()
        .find(|swarm| budget.spend(1) && swarm.id == owner)
        .map_or(Vec2::ZERO, |swarm| swarm.home);
    let home_cell = world_to_intent_cell(home);
    let cohort = living_defender_cohort_position(owner, home, state, budget);
    let nominal = world_to_intent_cell(cohort);
    candidate.anchor = nominal;
    if let Some(current) = current {
        for intent in current.desired.iter().take(MAX_PLAN_CELLS) {
            if !budget.spend(1) {
                return;
            }
            if intent.kind == IntentKind::Build
                && intent.cell != home_cell
                && (intent.cell - nominal).abs().max_element() <= 1
                && state
                    .grid
                    .cell(intent.cell)
                    .is_some_and(|cell| cell.has_owned(IntentKind::Build, owner))
                && pressure_anchor_is_open(owner, intent.cell, state, budget)
            {
                candidate.anchor = intent.cell;
                return;
            }
        }
    }
    let direction = (target - nominal).signum();
    if direction == IVec2::ZERO {
        return;
    }
    let lateral = IVec2::new(-direction.y, direction.x);
    let mut best = None;
    for anchor in [nominal, nominal + lateral, nominal - lateral] {
        if !pressure_anchor_is_open(owner, anchor, state, budget) {
            continue;
        }
        let anchor_position = intent_cell_center(anchor);
        let target_position = intent_cell_center(target);
        let travel = (cohort.distance(anchor_position) + anchor_position.distance(target_position))
            / crate::ZONE_BLOCK_SIZE;
        let exposure = route_exposure(cohort, anchor_position, state.terrain, budget)
            + route_exposure(anchor_position, target_position, state.terrain, budget);
        let cost = travel + exposure * tuning.strategy_pressure_anchor_exposure_cost;
        if best.is_none_or(|(_, best_cost)| cost < best_cost) {
            best = Some((anchor, cost));
        }
    }
    if let Some((anchor, _)) = best {
        candidate.anchor = anchor;
    }
}

pub(super) fn pressure_anchor_is_open(
    owner: SwarmId,
    anchor: IVec2,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> bool {
    if !state.grid.in_bounds(anchor) || !is_reachable(owner, state, anchor) {
        return false;
    }
    let anchor_position = intent_cell_center(anchor);
    for obstacle in state.terrain {
        if !budget.spend(1) {
            return false;
        }
        if !obstacle.admits_body(anchor_position) {
            return false;
        }
    }
    true
}

pub(super) fn living_defender_cohort_position(
    owner: SwarmId,
    home: Vec2,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Vec2 {
    let mut defenders = Vec::with_capacity(state.bots.len().min(MAX_BOTS));
    let mut xs = Vec::with_capacity(state.bots.len().min(MAX_BOTS));
    let mut ys = Vec::with_capacity(state.bots.len().min(MAX_BOTS));
    for bot in state.bots.iter().take(MAX_BOTS) {
        if !budget.spend(1) {
            return home;
        }
        if bot.owner == owner
            && bot.kind == NanobotType::Defender
            && bot.health > 0
            && bot.position.is_finite()
        {
            defenders.push((bot.id, bot.position));
            xs.push(bot.position.x);
            ys.push(bot.position.y);
        }
    }
    if xs.is_empty() {
        return home;
    }

    let sort_passes = usize::BITS.saturating_sub(xs.len().leading_zeros()) as usize;
    if !budget.spend(xs.len().saturating_mul(sort_passes).saturating_mul(2)) {
        return home;
    }
    xs.sort_unstable_by(f32::total_cmp);
    ys.sort_unstable_by(f32::total_cmp);
    let median = |values: &[f32]| {
        let middle = values.len() / 2;
        if values.len().is_multiple_of(2) {
            values[middle - 1] * 0.5 + values[middle] * 0.5
        } else {
            values[middle]
        }
    };
    let median = Vec2::new(median(&xs), median(&ys));
    let mut representative = None;
    for (id, position) in defenders {
        if !budget.spend(1) {
            return home;
        }
        let order = (
            position.distance_squared(median),
            position.distance_squared(home),
            id,
        );
        if representative.is_none_or(|(_, best_order): (Vec2, (f32, f32, u64))| {
            order
                .0
                .total_cmp(&best_order.0)
                .then_with(|| order.1.total_cmp(&best_order.1))
                .then_with(|| order.2.cmp(&best_order.2))
                .is_lt()
        }) {
            representative = Some((position, order));
        }
    }
    representative.map_or(home, |(position, _)| position)
}

pub(super) fn validate_plan(
    plan: &IntentPlan,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<PlanValidation, ()> {
    if plan
        .desired
        .iter()
        .any(|intent| !is_reachable(owner, state, intent.cell))
    {
        return Ok(PlanValidation {
            invalidation: Some("existing Intent Plan left the swarm-reachable map".to_string()),
            ..PlanValidation::default()
        });
    }
    if let Some(id) = plan.primary_deposit
        && !deposit_is_available(id, state, budget)?
    {
        return Ok(PlanValidation {
            invalidation: Some(format!(
                "primary Resource Deposit {id} was exhausted or lost"
            )),
            ..PlanValidation::default()
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
                ..PlanValidation::default()
            });
        }
    }
    let mut observed_attack_targets = Vec::with_capacity(plan.attack_targets.len());
    let mut covered_attack_progress = false;
    let mut covered_attack_movement = false;
    let mut all_attack_targets_covered = !plan.attack_targets.is_empty();
    for target in &plan.attack_targets {
        let Some(observed) = observe_attack_target(*target, owner, state, budget)? else {
            return Ok(PlanValidation {
                invalidation: Some(format!(
                    "target {} {} disappeared",
                    target.label(),
                    target.id
                )),
                ..PlanValidation::default()
            });
        };
        let progressed = observed.health < target.health;
        let covered = state
            .grid
            .cell(observed.cell)
            .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner));
        if observed.cell != target.cell && !covered {
            return Ok(PlanValidation {
                invalidation: Some(format!(
                    "target {} {} moved from {} to {}",
                    target.label(),
                    target.id,
                    target.cell,
                    observed.cell
                )),
                ..PlanValidation::default()
            });
        }
        covered_attack_progress |= progressed && covered;
        covered_attack_movement |= observed.cell != target.cell && covered;
        all_attack_targets_covered &= covered;
        observed_attack_targets.push(observed);
    }
    Ok(PlanValidation {
        invalidation: None,
        observed_attack_targets,
        covered_attack_progress,
        covered_attack_movement,
        all_attack_targets_covered,
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
                if bot.id == target.id && bot.owner != owner && bot.health > 0 {
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

pub(super) fn deposit_is_available(
    id: u64,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<bool, ()> {
    for deposit in state.deposits {
        if !budget.spend(1) {
            return Err(());
        }
        if deposit.id == id {
            return Ok(deposit.amount > 0);
        }
    }
    Ok(false)
}

pub(super) fn generate_candidates(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    current: Option<&IntentPlan>,
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
    let defend_target = (forces.friendly_defenders * 2 + 4).clamp(6, 20);
    let mut available = Vec::new();
    for deposit in state.deposits.iter().take(MAX_DEPOSITS) {
        if !budget.spend(1) {
            break;
        }
        if deposit.amount > 0 && is_reachable(owner, state, world_to_intent_cell(deposit.position))
        {
            let value = deposit_value(tuning, deposit, home, state, budget);
            available.push((deposit, value));
        }
    }
    let current_primary = current.and_then(|plan| plan.primary_deposit);
    let primary = current_primary
        .and_then(|id| {
            available
                .iter()
                .find(|(deposit, _)| deposit.id == id)
                .map(|(deposit, value)| (**deposit, *value))
        })
        .or_else(|| {
            available
                .iter()
                .max_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(deposit, value)| (**deposit, *value))
        });
    let economy = EconomySelection {
        primary: primary.map(|(deposit, _)| deposit),
        value: primary.map_or(0.0, |(_, value)| value),
    };
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
    let key_charger = state
        .structures
        .iter()
        .take(MAX_STRUCTURES)
        .find(|structure| {
            budget.spend(1)
                && structure.owner == owner
                && structure.kind == StructureKind::Charger
                && structure.health > 0
                && is_reachable(owner, state, world_to_intent_cell(structure.position))
        })
        .copied();
    let pressure_readiness_penalty = (tuning.strategy_readiness_supply_target - supply).max(0.0)
        * tuning.strategy_readiness_penalty_weight;

    let mut candidates = Vec::new();
    candidates.push(PlanCandidate {
        objective: Objective::HoldHome,
        economy,
        attack_targets: Vec::new(),
        anchor: home_cell,
        attack_target: None,
        key_charger,
        defend_target,
        score: economy.value
            + tuning.strategy_hold_base
            + supply.min(tuning.strategy_hold_supply_cap) * tuning.strategy_hold_supply_weight
            + forces.friendly_strength * tuning.strategy_hold_force_weight,
    });

    for &(deposit, value) in &available {
        if !budget.spend(1) {
            break;
        }
        let target = world_to_intent_cell(deposit.position);
        let scarcity = (tuning.strategy_scarcity_supply_target - supply).max(0.0)
            * tuning.strategy_scarcity_weight;
        let score = tuning.strategy_secure_base
            + value
            + scarcity
            + forces.friendly_strength * tuning.strategy_secure_force_weight
            - forces.enemy_strength * tuning.strategy_secure_enemy_weight;
        candidates.push(PlanCandidate {
            objective: Objective::SecureDeposit(deposit.id),
            economy: EconomySelection {
                primary: Some(*deposit),
                value,
            },
            attack_targets: Vec::new(),
            anchor: target,
            attack_target: None,
            key_charger,
            defend_target,
            score,
        });
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
        let anchor = midpoint_cell(home_cell, target);
        if is_reachable(owner, state, target) && is_reachable(owner, state, anchor) {
            let distance = home.distance(enemy_home) / crate::ZONE_BLOCK_SIZE;
            let score = economy.value
                + tuning.strategy_pressure_base
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
                objective: Objective::ForwardPressure(anchor),
                economy,
                attack_targets: Vec::new(),
                anchor,
                attack_target: Some(target),
                key_charger,
                defend_target,
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
        let anchor = midpoint_cell(home_cell, target);
        if !is_reachable(owner, state, target) || !is_reachable(owner, state, anchor) {
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
        let score = economy.value
            + tuning.strategy_raid_base
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
            economy,
            attack_targets: vec![AttackObligation {
                kind: AttackTargetKind::Structure,
                id: structure.id,
                cell: target,
                health: structure.health,
            }],
            anchor,
            attack_target: Some(target),
            key_charger,
            defend_target,
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
        let anchor = midpoint_cell(home_cell, target);
        if !is_reachable(owner, state, target) || !is_reachable(owner, state, anchor) {
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
        let score = economy.value
            + cleanup_priority
            + danger
            + vulnerability
            + forces.friendly_strength * tuning.strategy_hunt_force_weight
            - cleanup_readiness_penalty
            - distance * tuning.strategy_hunt_distance_cost
            - route_exposure(home, bot.position, state.terrain, budget)
                * tuning.strategy_hunt_exposure_cost;
        candidates.push(PlanCandidate {
            objective: Objective::HuntRemnant(bot.id),
            economy,
            attack_targets: vec![AttackObligation {
                kind: AttackTargetKind::Nanobot,
                id: bot.id,
                cell: target,
                health: bot.health,
            }],
            anchor,
            attack_target: Some(target),
            key_charger,
            defend_target,
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

pub(super) fn deposit_value(
    tuning: &AutonomyTuning,
    deposit: &DepositState,
    home: Vec2,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> f32 {
    let travel = home.distance(deposit.position) / crate::ZONE_BLOCK_SIZE;
    let exposure = route_exposure(home, deposit.position, state.terrain, budget);
    (deposit.amount as f32 + 1.0).ln() * tuning.strategy_deposit_amount_weight
        - travel * tuning.strategy_deposit_distance_cost
        - exposure * tuning.strategy_deposit_exposure_cost
}

fn health_deficit(health: u32, max_health: u32) -> f32 {
    100.0 * max_health.saturating_sub(health) as f32 / max_health.max(1) as f32
}
