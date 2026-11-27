use super::economy::{EconomyPlan, deposit_region, validate_owned_deposits};
use super::{
    lifecycle::WorkBudget,
    materialize::{cell_is_reachable, world_to_intent_cell},
    *,
};
use crate::nanobot::{cell_overlaps_circle, deposit_candidate_cells};

const SWARM_ELIMINATION_BONUS: f32 = 80.0;

fn is_target_reachable(
    owner: SwarmId,
    state: &GameState<'_>,
    position: Vec2,
    budget: &mut WorkBudget,
) -> Result<bool, ()> {
    let Some(map) = state.reachability else {
        return Ok(true);
    };
    map.position_is_reachable_with_budget(owner, position, state.grid, &mut || budget.spend(1))
}

pub(super) fn validate_plan(
    plan: &IntentPlan,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<PlanValidation, ()> {
    let mut unreachable = HashSet::new();
    if let Some(map) = state.reachability
        && (plan.grid_size != IVec2::new(state.grid.width(), state.grid.height())
            || plan
                .reachability
                .as_ref()
                .is_none_or(|previous| !map.same_boundary(previous)))
    {
        if !budget.spend(plan.desired.len()) {
            return Err(());
        }
        for &intent in &plan.desired {
            if !map.contains_with_budget(owner, intent.cell, state.grid, &mut || budget.spend(1))? {
                unreachable.insert(intent);
            }
        }
    }
    let validation = PlanValidation {
        lost_deposit: validate_owned_deposits(
            &plan.owned_deposits,
            owner,
            state,
            &mut unreachable,
            budget,
        )?,
        ..PlanValidation::default()
    };
    if !unreachable.is_empty() {
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
                    && is_target_reachable(owner, state, structure.position, budget)?
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
                    && is_target_reachable(owner, state, bot.position, budget)?
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
    economy: &EconomyPlan,
    denied_deposits: &mut BTreeSet<(u64, SwarmId)>,
    budget: &mut WorkBudget,
) -> Vec<PlanCandidate> {
    let owner_state = state.swarms.iter().find(|swarm| {
        let within_budget = budget.spend(1);
        within_budget && swarm.id == owner
    });
    let home = owner_state.map_or(Vec2::ZERO, |swarm| swarm.home);
    let home_cell = world_to_intent_cell(home);
    let base = PlanCandidate::hold_home(home_cell);
    let mut candidates = vec![base.clone()];
    if let Some((deposit, target)) = economy.threatened {
        candidates.push(PlanCandidate {
            objective: Objective::DefendDeposit(deposit),
            attack_target: Some(target),
            ..base
        });
        return candidates;
    }

    for &(deposit, cell, approach) in &economy.contested_deposits {
        candidates.push(PlanCandidate {
            objective: Objective::ContestDeposit(deposit),
            attack_target: Some(cell),
            approach: Some(approach),
            score: -home_cell.as_vec2().distance(cell.as_vec2()),
            ..base
        });
    }
    let forces = estimate_forces(tuning, owner, state, budget);
    let own_income = owner_state.map_or(0.0, |swarm| swarm.mineral_income_per_second);
    let mut hostile_bots = BTreeSet::new();
    let mut recoverable = BTreeSet::new();
    let mut completed_structures = BTreeMap::<SwarmId, usize>::new();
    for bot in state.bots {
        if !budget.spend(1) {
            return candidates;
        }
        if bot.owner != owner && bot.health > 0 {
            hostile_bots.insert(bot.owner);
        }
    }
    let mut supply = owner_state.map_or(0.0, |swarm| {
        swarm.minerals as f32 * tuning.strategy_ledger_supply_weight
    });
    for structure in state.structures {
        if !budget.spend(1) {
            return candidates;
        }
        if structure.owner != owner && structure.kind.counts_for_elimination() {
            *completed_structures.entry(structure.owner).or_default() += 1;
        }
        if structure.health == 0 || structure.kind == StructureKind::Planned {
            continue;
        }
        if structure.owner != owner {
            if structure.kind == StructureKind::Facility
                && (structure.minerals > 0 || hostile_bots.contains(&structure.owner))
            {
                recoverable.insert(structure.owner);
            }
        } else if structure.operational
            && matches!(
                structure.kind,
                StructureKind::Source | StructureKind::Sink | StructureKind::Charger
            )
        {
            supply += structure.minerals as f32 * tuning.strategy_buffer_supply_weight;
        }
    }
    let readiness_penalty = (tuning.strategy_readiness_supply_target - supply).max(0.0)
        * tuning.strategy_readiness_penalty_weight;
    let offensive_value = tuning.strategy_raid_base
        + supply.min(tuning.strategy_raid_supply_cap) * tuning.strategy_raid_supply_weight
        + forces.friendly_strength * tuning.strategy_raid_force_weight
        - forces.enemy_strength * tuning.strategy_raid_enemy_weight
        - readiness_penalty;
    for swarm in state.swarms {
        if !budget.spend(1) {
            return candidates;
        }
        if swarm.id == owner {
            continue;
        }
        if swarm.mineral_income_per_second > 0.0 {
            recoverable.insert(swarm.id);
        }
        if own_income - swarm.mineral_income_per_second > tuning.strategy_home_income_lead
            && home_has_targets(swarm, state, budget)
        {
            let Ok(reachable) = is_target_reachable(owner, state, swarm.home, budget) else {
                return candidates;
            };
            if !reachable {
                continue;
            }
            candidates.push(PlanCandidate {
                objective: Objective::PushHome(swarm.id),
                attack_target: Some(world_to_intent_cell(swarm.home)),
                score: offensive_value
                    - home.distance(swarm.home) / crate::ZONE_BLOCK_SIZE
                        * tuning.strategy_raid_distance_cost,
                ..base
            });
        }
    }

    let mut occupied_mines = BTreeSet::new();
    let mut nearby_mines = Vec::new();
    for structure in state.structures {
        if !budget.spend(1) {
            return candidates;
        }
        if structure.owner == owner
            || structure.health == 0
            || structure.kind == StructureKind::Planned
        {
            continue;
        }
        let attack = AttackObligation {
            kind: AttackTargetKind::Structure,
            id: structure.id,
            cell: world_to_intent_cell(structure.position),
            health: structure.health,
        };
        nearby_mines.clear();
        let mine = mine_target(
            economy,
            owner,
            structure.owner,
            structure.position,
            state,
            &mut nearby_mines,
            budget,
        );
        let finishes_match = structure.kind.counts_for_elimination()
            && completed_structures.get(&structure.owner) == Some(&1);
        let objective = if let Some((deposit, _)) = mine {
            Objective::RaidDeposit {
                deposit,
                target: structure.id,
            }
        } else if finishes_match || !recoverable.contains(&structure.owner) {
            Objective::AttackStructure(structure.id)
        } else {
            continue;
        };
        let Ok(reachable) = is_target_reachable(owner, state, structure.position, budget) else {
            return candidates;
        };
        if !reachable {
            continue;
        }
        occupied_mines.extend(nearby_mines.iter().copied());
        let strategic_value = match structure.kind {
            StructureKind::Charger => tuning.strategy_raid_charger_value,
            StructureKind::Source | StructureKind::Sink => tuning.strategy_raid_stockpile_value,
            StructureKind::Facility => tuning.strategy_raid_facility_value,
            StructureKind::Planned | StructureKind::Other => 0.0,
        };
        let distance = home.distance(structure.position) / crate::ZONE_BLOCK_SIZE;
        let elimination_bonus = if finishes_match {
            SWARM_ELIMINATION_BONUS
        } else {
            0.0
        };
        let score = offensive_value
            + strategic_value
            + elimination_bonus
            + health_deficit(structure.health, structure.max_health)
                * tuning.strategy_raid_vulnerability_weight
            + structure.minerals as f32 * tuning.strategy_raid_cargo_weight
            - distance * tuning.strategy_raid_distance_cost;
        candidates.push(PlanCandidate {
            objective,
            attack: Some(attack),
            attack_target: Some(attack.cell),
            approach: Some(attack_approach(structure.position)),
            score,
            protected_home: mine.and_then(|(_, home_cell)| home_cell),
            ..base
        });
    }

    for bot in state.bots {
        if !budget.spend(1) {
            return candidates;
        }
        if bot.owner == owner || bot.health == 0 {
            continue;
        }
        nearby_mines.clear();
        let mine = mine_target(
            economy,
            owner,
            bot.owner,
            bot.position,
            state,
            &mut nearby_mines,
            budget,
        );
        let objective = if let Some((deposit, _)) = mine {
            Objective::RaidDeposit {
                deposit,
                target: bot.id,
            }
        } else if !recoverable.contains(&bot.owner) {
            Objective::HuntRemnant(bot.id)
        } else {
            continue;
        };
        let Ok(reachable) = is_target_reachable(owner, state, bot.position, budget) else {
            return candidates;
        };
        if !reachable {
            continue;
        }
        occupied_mines.extend(nearby_mines.iter().copied());
        let attack = AttackObligation {
            kind: AttackTargetKind::Nanobot,
            id: bot.id,
            cell: world_to_intent_cell(bot.position),
            health: bot.health,
        };
        let danger = match bot.kind {
            NanobotType::Defender => tuning.strategy_hunt_defender_value,
            NanobotType::Hauler => tuning.strategy_hunt_hauler_value,
            NanobotType::Worker => tuning.strategy_hunt_worker_value,
        };
        let distance = home.distance(bot.position) / crate::ZONE_BLOCK_SIZE;
        let score = offensive_value
            + danger
            + health_deficit(bot.health, bot.max_health)
                * tuning.strategy_hunt_vulnerability_weight
            - distance * tuning.strategy_hunt_distance_cost;
        candidates.push(PlanCandidate {
            objective,
            attack: Some(attack),
            attack_target: Some(attack.cell),
            approach: Some(attack_approach(bot.position)),
            score,
            protected_home: mine.and_then(|(_, home_cell)| home_cell),
            ..base
        });
    }
    for (deposit, enemy) in &economy.enemy_deposits {
        if !budget.spend(1) {
            return candidates;
        }
        let claim = (deposit.id, *enemy);
        if occupied_mines.contains(&deposit.id) {
            denied_deposits.remove(&claim);
            continue;
        }
        if denied_deposits.contains(&claim) || !economy.enemy_gather_claims.contains(&claim) {
            continue;
        }
        let protected_home = swarm_home_cell(*enemy, state, budget);
        let region = deposit_region(deposit);
        for cell in deposit_candidate_cells(state.grid, deposit.position, deposit.radius) {
            if !budget.spend(1) {
                return candidates;
            }
            if Some(cell) == protected_home
                || !cell_overlaps_circle(cell, deposit.position, deposit.radius)
                || !state
                    .grid
                    .cell(cell)
                    .is_some_and(|intent| intent.has_owned(IntentKind::Gather, *enemy))
            {
                continue;
            }
            let interaction = region.within_cell(cell);
            if let Some(map) = state.reachability {
                match cell_is_reachable(owner, state, cell, budget) {
                    Some(true) => {}
                    Some(false) => continue,
                    None => return candidates,
                }
                if state.route_query.is_none() {
                    match map.interaction_is_reachable_with_budget(owner, interaction, &mut || {
                        budget.spend(1)
                    }) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(()) => return candidates,
                    }
                }
            }
            candidates.push(PlanCandidate {
                objective: Objective::DenyDeposit(deposit.id),
                attack_target: Some(cell),
                approach: Some(RouteGoal::Interaction(interaction)),
                score: offensive_value
                    - home_cell.as_vec2().distance(cell.as_vec2())
                        * tuning.strategy_raid_distance_cost,
                protected_home,
                ..base
            });
        }
    }
    candidates
}

fn attack_approach(position: Vec2) -> RouteGoal {
    RouteGoal::Range {
        center: position,
        radius: crate::ZONE_BLOCK_SIZE / 2.0,
    }
}

fn home_has_targets(swarm: &SwarmState, state: &GameState<'_>, budget: &mut WorkBudget) -> bool {
    let near_home =
        |position: Vec2| position.distance_squared(swarm.home) <= crate::ZONE_BLOCK_SIZE.powi(2);
    state
        .structures
        .iter()
        .take_while(|_| budget.spend(1))
        .any(|structure| {
            structure.owner == swarm.id
                && structure.health > 0
                && structure.kind != StructureKind::Planned
                && near_home(structure.position)
        })
        || state
            .bots
            .iter()
            .take_while(|_| budget.spend(1))
            .any(|bot| bot.owner == swarm.id && bot.health > 0 && near_home(bot.position))
}

fn swarm_home_cell(
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<IVec2> {
    state
        .swarms
        .iter()
        .take_while(|_| budget.spend(1))
        .find(|swarm| swarm.id == owner)
        .map(|swarm| world_to_intent_cell(swarm.home))
}

fn mine_target(
    economy: &EconomyPlan,
    attacker: SwarmId,
    owner: SwarmId,
    position: Vec2,
    state: &GameState<'_>,
    nearby_mines: &mut Vec<u64>,
    budget: &mut WorkBudget,
) -> Option<(u64, Option<IVec2>)> {
    let home_cell = swarm_home_cell(owner, state, budget);
    if home_cell == Some(world_to_intent_cell(position)) {
        return None;
    }
    let mut closest = None;
    for (deposit, mine_owner) in &economy.enemy_deposits {
        if !budget.spend(1) {
            return None;
        }
        if *mine_owner != owner {
            continue;
        }
        let distance = position.distance_squared(deposit.position);
        if distance <= (deposit.radius + crate::ZONE_BLOCK_SIZE).powi(2)
            && deposit_is_reachable(attacker, deposit, state, budget)?
        {
            nearby_mines.push(deposit.id);
            if closest.is_none_or(|(_, best_distance)| distance < best_distance) {
                closest = Some((deposit.id, distance));
            }
        }
    }
    closest.map(|(id, _)| (id, home_cell))
}

fn deposit_is_reachable(
    owner: SwarmId,
    deposit: &DepositState,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Option<bool> {
    for cell in deposit_candidate_cells(state.grid, deposit.position, deposit.radius) {
        if !budget.spend(1) {
            return None;
        }
        if cell_overlaps_circle(cell, deposit.position, deposit.radius)
            && cell_is_reachable(owner, state, cell, budget)?
        {
            return Some(true);
        }
    }
    Some(false)
}

fn estimate_forces(
    tuning: &AutonomyTuning,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> ForceEstimate {
    let mut estimate = ForceEstimate::default();
    for bot in state.bots {
        if !budget.spend(1) {
            break;
        }
        if bot.kind != NanobotType::Defender || bot.health == 0 {
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
            estimate.friendly_strength += strength;
        } else {
            estimate.enemy_strength += strength;
        }
    }
    estimate
}

fn health_deficit(health: u32, max_health: u32) -> f32 {
    100.0 * max_health.saturating_sub(health) as f32 / max_health.max(1) as f32
}
