//! Pure strategic intent planning.
//!
//! Controllers receive an immutable snapshot of the exact simulation state and
//! return edits which the runtime applies only for [`Controller::owner`]. The
//! module never writes ECS state or chooses work for individual nanobots.

use std::collections::BTreeSet;

use bevy::prelude::{IVec2, Vec2};

use crate::{
    intent::{IntentGrid, IntentKind},
    nanobot::{NanobotType, SwarmId},
    navigation::Obstacle,
};

/// Maximum accounted planner operations shared by all decisions in one review window.
pub const PLANNING_WORK_BUDGET: usize = 100_000;
pub const REVIEW_PERIOD_TICKS: u64 = crate::SIMULATION_HZ as u64 / 2;

const EDIT_WORK_RESERVE: usize = 4_096;
const MAX_BOTS: usize = 256;
const MAX_STRUCTURES: usize = 128;
const MAX_DEPOSITS: usize = 24;
const MAX_ACTIVE_CELLS: usize = 256;
const MAX_PLAN_CELLS: usize = 96;
const SWITCH_MARGIN: f32 = 1.15;
const MIN_SECOND_FRONT_DEFENDERS: usize = 4;
const MIN_FRONT_HEALTH: u32 = crate::nanobot::NANOBOT_DEFAULT_MAX_HEALTH / 2;
const MIN_FRONT_CHARGE: f32 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwarmState {
    pub id: SwarmId,
    pub home: Vec2,
    pub minerals: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotState {
    pub id: u64,
    pub owner: SwarmId,
    pub kind: NanobotType,
    pub position: Vec2,
    pub health: u32,
    pub charge: f32,
    pub cargo: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureKind {
    Facility,
    Source,
    Sink,
    Charger,
    Planned,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StructureState {
    pub id: u64,
    pub owner: SwarmId,
    pub position: Vec2,
    pub kind: StructureKind,
    pub health: u32,
    pub minerals: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepositState {
    pub id: u64,
    pub position: Vec2,
    pub amount: u32,
    pub radius: f32,
}

#[derive(Debug)]
pub struct GameState<'a> {
    pub grid: &'a IntentGrid,
    pub swarms: &'a [SwarmState],
    pub bots: &'a [BotState],
    pub structures: &'a [StructureState],
    pub deposits: &'a [DepositState],
    pub terrain: &'a [Obstacle],
    pub tick: u64,
    pub finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentEdit {
    pub cell: IVec2,
    pub kind: IntentKind,
    pub paint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub edits: Vec<IntentEdit>,
    pub explanation: String,
    pub reviewed: bool,
    pub work_units: usize,
}

impl Decision {
    fn idle(explanation: impl Into<String>) -> Self {
        Self {
            edits: Vec::new(),
            explanation: explanation.into(),
            reviewed: false,
            work_units: 0,
        }
    }
}

#[derive(Debug)]
pub struct Controller {
    owner: SwarmId,
    policy: Policy,
}

#[derive(Debug)]
enum Policy {
    Adaptive(AdaptiveController),
    Timed(TimedController),
}

#[derive(Debug, Default)]
struct AdaptiveController {
    last_review_tick: Option<u64>,
    work_window_start_tick: Option<u64>,
    work_used: usize,
    retained_covered_movement: bool,
    covered_stall_grace_used: bool,
    current: Option<IntentPlan>,
}

#[derive(Debug, Clone)]
struct IntentPlan {
    objective: Objective,
    primary_deposit: Option<u64>,
    attack_targets: Vec<AttackObligation>,
    key_charger: Option<u64>,
    desired: Vec<DesiredIntent>,
}

#[derive(Debug, Default)]
struct PlanValidation {
    invalidation: Option<String>,
    observed_attack_targets: Vec<AttackObligation>,
    covered_attack_progress: bool,
    covered_attack_movement: bool,
    all_attack_targets_covered: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct EconomySelection {
    primary: Option<DepositState>,
    value: f32,
}

#[derive(Debug, Clone)]
struct PlanCandidate {
    objective: Objective,
    economy: EconomySelection,
    attack_targets: Vec<AttackObligation>,
    anchor: IVec2,
    attack_target: Option<IVec2>,
    key_charger: Option<StructureState>,
    defend_target: usize,
    score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttackTargetKind {
    Structure,
    Nanobot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttackObligation {
    kind: AttackTargetKind,
    id: u64,
    cell: IVec2,
    health: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Objective {
    HoldHome,
    SecureDeposit(u64),
    ForwardPressure(IVec2),
    RaidLogistics(u64),
    HuntRemnant(u64),
}

impl Objective {
    fn label(self) -> String {
        match self {
            Self::HoldHome => "hold the home economy".to_string(),
            Self::SecureDeposit(id) => format!("secure Resource Deposit {id}"),
            Self::ForwardPressure(cell) => format!("establish a supplied front at {cell}"),
            Self::RaidLogistics(id) => format!("attack enemy logistics structure {id}"),
            Self::HuntRemnant(id) => format!("hunt remaining Nanobot {id}"),
        }
    }
}

impl IntentPlan {
    fn is_coordinated(&self) -> bool {
        [IntentKind::Build, IntentKind::Defend, IntentKind::Corridor]
            .into_iter()
            .all(|kind| self.desired.iter().any(|intent| intent.kind == kind))
            && (self.primary_deposit.is_none()
                || self
                    .desired
                    .iter()
                    .any(|intent| intent.kind == IntentKind::Gather))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesiredIntent {
    cell: IVec2,
    kind: IntentKind,
}

#[derive(Debug)]
struct WorkBudget {
    used: usize,
    call_start: usize,
    limit: usize,
    exhausted: bool,
}

impl WorkBudget {
    fn planning(already_used: usize) -> Self {
        Self {
            used: already_used,
            call_start: already_used,
            limit: PLANNING_WORK_BUDGET - EDIT_WORK_RESERVE,
            exhausted: false,
        }
    }

    fn release_edit_reserve(&mut self) {
        self.limit = PLANNING_WORK_BUDGET;
    }

    fn spend(&mut self, amount: usize) -> bool {
        if self.used.saturating_add(amount) > self.limit {
            self.exhausted = true;
            false
        } else {
            self.used += amount;
            true
        }
    }

    fn call_work(&self) -> usize {
        self.used.saturating_sub(self.call_start)
    }
}

#[derive(Debug, Default)]
struct ForceEstimate {
    friendly_defenders: usize,
    friendly_strength: f32,
    enemy_strength: f32,
    low_charge_defenders: usize,
}

#[derive(Debug)]
struct TimedController {
    assault_cell: IVec2,
    target_cell: IVec2,
    ticks_until_advance: u32,
    period: u32,
}

impl Controller {
    pub fn adaptive(owner: SwarmId) -> Self {
        Self {
            owner,
            policy: Policy::Adaptive(AdaptiveController::default()),
        }
    }

    pub fn timed(
        owner: SwarmId,
        assault_cell: IVec2,
        target_cell: IVec2,
        initial_delay_ticks: u32,
        period: u32,
    ) -> Self {
        Self {
            owner,
            policy: Policy::Timed(TimedController {
                assault_cell,
                target_cell,
                ticks_until_advance: initial_delay_ticks,
                period: period.max(1),
            }),
        }
    }

    pub fn owner(&self) -> SwarmId {
        self.owner
    }

    pub fn decide(&mut self, state: &GameState<'_>) -> Decision {
        if state.finished {
            return Decision::idle("match finished");
        }
        let owner = self.owner;
        match &mut self.policy {
            Policy::Adaptive(adaptive) => adaptive.decide(owner, state),
            Policy::Timed(timed) => timed.decide(),
        }
    }
}

impl AdaptiveController {
    fn decide(&mut self, owner: SwarmId, state: &GameState<'_>) -> Decision {
        let window_start = state.tick - state.tick % REVIEW_PERIOD_TICKS;
        if self.work_window_start_tick != Some(window_start) {
            self.work_window_start_tick = Some(window_start);
            self.work_used = 0;
        }
        let mut budget = WorkBudget::planning(self.work_used);
        let validation = match self
            .current
            .as_ref()
            .map(|plan| validate_plan(plan, owner, state, &mut budget))
            .transpose()
        {
            Ok(validation) => validation.unwrap_or_default(),
            Err(()) => {
                self.work_used = budget.used;
                return Decision {
                    edits: Vec::new(),
                    explanation:
                        "planning allowance exhausted while validating the current Intent Plan; deferred until the next review window"
                            .to_string(),
                    reviewed: false,
                    work_units: budget.call_work(),
                };
            }
        };
        let PlanValidation {
            mut invalidation,
            observed_attack_targets,
            covered_attack_progress,
            covered_attack_movement,
            all_attack_targets_covered,
        } = validation;
        let regular_review = self.last_review_tick.is_none_or(|last| {
            state.tick < last || state.tick.saturating_sub(last) >= REVIEW_PERIOD_TICKS
        });
        if invalidation.is_none() && self.retained_covered_movement && !all_attack_targets_covered {
            invalidation =
                Some("a retained attack target left existing owned Defend coverage".to_string());
        }
        let reviewed_supportable_fronts = if invalidation.is_none()
            && regular_review
            && (self.retained_covered_movement || covered_attack_movement)
            && !observed_attack_targets.is_empty()
        {
            Some(supportable_attack_fronts(owner, state, &mut budget))
        } else {
            None
        };
        let retained_fronts_are_supportable = reviewed_supportable_fronts
            .is_none_or(|fronts| observed_attack_targets.len() <= fronts);
        let retained_fronts_fill_capacity = reviewed_supportable_fronts
            .is_none_or(|fronts| observed_attack_targets.len() >= fronts);
        let retains_covered_progress =
            covered_attack_progress && (covered_attack_movement || self.retained_covered_movement);
        let retains_covered_movement_between_reviews = covered_attack_movement && !regular_review;
        if invalidation.is_none()
            && (retains_covered_progress || retains_covered_movement_between_reviews)
            && all_attack_targets_covered
            && retained_fronts_are_supportable
            && retained_fronts_fill_capacity
        {
            if let Some(current) = self.current.as_mut() {
                if regular_review {
                    current.attack_targets = observed_attack_targets;
                } else {
                    for (target, observed) in current
                        .attack_targets
                        .iter_mut()
                        .zip(observed_attack_targets)
                    {
                        target.cell = observed.cell;
                    }
                }
            }
            self.retained_covered_movement = true;
            if covered_attack_progress {
                self.covered_stall_grace_used = false;
            }
            if regular_review {
                self.last_review_tick = Some(state.tick);
            }
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: if covered_attack_progress {
                    "progressing attack remains within existing owned Defend coverage"
                } else {
                    "moving attack remains within existing owned Defend coverage"
                }
                .to_string(),
                reviewed: regular_review,
                work_units: budget.call_work(),
            };
        }
        if invalidation.is_none()
            && regular_review
            && (self.retained_covered_movement || covered_attack_movement)
            && !covered_attack_progress
            && all_attack_targets_covered
            && retained_fronts_are_supportable
            && !self.covered_stall_grace_used
        {
            if let Some(current) = self.current.as_mut() {
                current.attack_targets = observed_attack_targets;
            }
            self.covered_stall_grace_used = true;
            self.last_review_tick = Some(state.tick);
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: "covered attack received one stalled review before plan reassessment"
                    .to_string(),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        if !regular_review && invalidation.is_none() {
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: "current Intent Plan remains between reviews".to_string(),
                reviewed: false,
                work_units: budget.call_work(),
            };
        }

        let candidates = generate_candidates(owner, state, self.current.as_ref(), &mut budget);
        if budget.exhausted {
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation:
                    "planning allowance exhausted during review; retained the current Intent Plan until the next review window"
                        .to_string(),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        let reassessing_spare_front = invalidation.is_none()
            && regular_review
            && retains_covered_progress
            && all_attack_targets_covered
            && retained_fronts_are_supportable
            && !retained_fronts_fill_capacity;
        let has_fresh_attack_target = candidates.iter().any(|candidate| {
            candidate.attack_targets.iter().any(|candidate_target| {
                !observed_attack_targets.iter().any(|retained| {
                    retained.kind == candidate_target.kind && retained.id == candidate_target.id
                })
            })
        });
        if reassessing_spare_front && !has_fresh_attack_target {
            if let Some(current) = self.current.as_mut() {
                current.attack_targets = observed_attack_targets;
            }
            self.covered_stall_grace_used = false;
            self.last_review_tick = Some(state.tick);
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: "progressing covered attack found no useful additional front"
                    .to_string(),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        let Some(best) = candidates
            .iter()
            .max_by(|left, right| left.score.total_cmp(&right.score))
            .cloned()
        else {
            self.work_used = budget.used;
            self.last_review_tick = Some(state.tick);
            return Decision {
                edits: Vec::new(),
                explanation: "review found no legal Intent Plan".to_string(),
                reviewed: true,
                work_units: budget.call_work(),
            };
        };

        let (mut selected, mut explanation) = if let Some(reason) = invalidation {
            let label = best.objective.label();
            (
                best,
                format!("{reason}; selected {label} after urgent review"),
            )
        } else if let Some(current) = self.current.as_ref() {
            let refreshed = candidates
                .iter()
                .find(|candidate| candidate.objective == current.objective);
            if let Some(refreshed) = refreshed
                && best.score
                    <= refreshed.score
                        + (refreshed.score - refreshed.economy.value).abs() * (SWITCH_MARGIN - 1.0)
                        + 10.0
            {
                (
                    refreshed.clone(),
                    format!(
                        "retained viable {}; no alternative was clearly better",
                        refreshed.objective.label()
                    ),
                )
            } else {
                let label = best.objective.label();
                (
                    best,
                    format!("selected {label} because it is clearly better than the current plan"),
                )
            }
        } else {
            let label = best.objective.label();
            (
                best,
                format!("selected {label} from economic, support, and combat alternatives"),
            )
        };
        let provisional_objective = selected.objective;
        let attack_explanation = coordinate_attack_targets(
            &mut selected,
            &candidates,
            self.current.as_ref(),
            owner,
            state,
            reviewed_supportable_fronts,
            &mut budget,
        );
        if selected.objective != provisional_objective {
            explanation = format!(
                "retained {} after attack-front coordination",
                selected.objective.label()
            );
        }
        adapt_pressure_anchor(
            &mut selected,
            self.current.as_ref(),
            owner,
            state,
            &mut budget,
        );
        if budget.exhausted {
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation:
                    "planning allowance exhausted while comparing pressure approaches; retained the current Intent Plan until the next review window"
                        .to_string(),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        let explanation = format!(
            "{explanation}; anchor {}; target {}{}",
            selected.anchor,
            selected.attack_target.unwrap_or(selected.anchor),
            attack_explanation
                .as_deref()
                .map_or(String::new(), |detail| format!("; {detail}"))
        );

        let selected = materialize_plan(selected, owner, state, &mut budget);
        if budget.exhausted || !selected.is_coordinated() {
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: format!(
                    "{explanation}; planning allowance exhausted during paint fulfillment, so the current Intent Plan was retained"
                ),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        budget.release_edit_reserve();
        let edits = intent_edits(owner, state.grid, &selected.desired, &mut budget);
        self.work_used = budget.used;
        self.last_review_tick = Some(state.tick);
        self.retained_covered_movement = false;
        self.covered_stall_grace_used = false;
        self.current = Some(selected);
        Decision {
            edits,
            explanation,
            reviewed: true,
            work_units: budget.call_work(),
        }
    }
}

fn coordinate_attack_targets(
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
        .unwrap_or_else(|| supportable_attack_fronts(owner, state, budget))
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

fn supportable_attack_fronts(
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
            && bot.health >= MIN_FRONT_HEALTH
            && bot.charge.is_finite()
            && bot.charge >= MIN_FRONT_CHARGE
        {
            ready_defenders += 1;
        }
    }
    if ready_defenders < MIN_SECOND_FRONT_DEFENDERS {
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

fn adapt_pressure_anchor(
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
                && pressure_anchor_is_open(intent.cell, state, budget)
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
        if !pressure_anchor_is_open(anchor, state, budget) {
            continue;
        }
        let anchor_position = intent_cell_center(anchor);
        let target_position = intent_cell_center(target);
        let travel = (cohort.distance(anchor_position) + anchor_position.distance(target_position))
            / crate::ZONE_BLOCK_SIZE;
        let exposure = route_exposure(cohort, anchor_position, state.terrain, budget)
            + route_exposure(anchor_position, target_position, state.terrain, budget);
        let cost = travel + exposure * 12.0;
        if best.is_none_or(|(_, best_cost)| cost < best_cost) {
            best = Some((anchor, cost));
        }
    }
    if let Some((anchor, _)) = best {
        candidate.anchor = anchor;
    }
}

fn pressure_anchor_is_open(anchor: IVec2, state: &GameState<'_>, budget: &mut WorkBudget) -> bool {
    if !state.grid.in_bounds(anchor) {
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

fn living_defender_cohort_position(
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

fn validate_plan(
    plan: &IntentPlan,
    owner: SwarmId,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> Result<PlanValidation, ()> {
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

fn observe_attack_target(
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

fn deposit_is_available(
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

fn generate_candidates(
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
    let forces = estimate_forces(owner, state, budget);
    let defend_target = (forces.friendly_defenders * 2 + 4).clamp(6, 20);
    let mut available = Vec::new();
    for deposit in state.deposits.iter().take(MAX_DEPOSITS) {
        if !budget.spend(1) {
            break;
        }
        if deposit.amount > 0 {
            let value = deposit_value(deposit, home, state, budget);
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
    let supply = owner_state.map_or(0.0, |swarm| swarm.minerals as f32 * 0.1)
        + state
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
            .map(|structure| structure.minerals as f32 * 0.05)
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
        })
        .copied();
    let pressure_readiness_penalty = (80.0 - supply).max(0.0) * 2.0;

    let mut candidates = Vec::new();
    candidates.push(PlanCandidate {
        objective: Objective::HoldHome,
        economy,
        attack_targets: Vec::new(),
        anchor: home_cell,
        attack_target: None,
        key_charger,
        defend_target,
        score: economy.value + 25.0 + supply.min(100.0) * 0.1 + forces.friendly_strength * 0.02,
    });

    for &(deposit, value) in &available {
        if !budget.spend(1) {
            break;
        }
        let target = world_to_intent_cell(deposit.position);
        let scarcity = (60.0 - supply).max(0.0) * 0.4;
        let score = 75.0 + value + scarcity + forces.friendly_strength * 0.025
            - forces.enemy_strength * 0.01;
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
        let distance = home.distance(enemy_home) / crate::ZONE_BLOCK_SIZE;
        let score =
            economy.value + 160.0 + supply.min(120.0) * 0.7 + forces.friendly_strength * 0.04
                - forces.enemy_strength * 0.035
                - forces.low_charge_defenders as f32 * 18.0
                - pressure_readiness_penalty
                - distance * 2.0
                - route_exposure(home, enemy_home, state.terrain, budget) * 12.0;
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
        let strategic_value = match structure.kind {
            StructureKind::Charger => 95.0,
            StructureKind::Source | StructureKind::Sink => 80.0,
            StructureKind::Facility => 60.0,
            StructureKind::Planned | StructureKind::Other => 0.0,
        };
        let distance = home.distance(structure.position) / crate::ZONE_BLOCK_SIZE;
        let vulnerability = 100_u32.saturating_sub(structure.health.min(100)) as f32 * 0.8;
        let score = economy.value
            + 220.0
            + strategic_value
            + vulnerability
            + supply.min(120.0) * 0.7
            + structure.minerals as f32 * 0.15
            + forces.friendly_strength * 0.045
            - forces.enemy_strength * 0.04
            - pressure_readiness_penalty
            - distance * 3.0
            - route_exposure(home, structure.position, state.terrain, budget) * 14.0;
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
        let danger = match bot.kind {
            NanobotType::Defender => 60.0,
            NanobotType::Hauler => 25.0,
            NanobotType::Worker => 15.0,
        };
        let distance = home.distance(bot.position) / crate::ZONE_BLOCK_SIZE;
        let vulnerability = 100_u32.saturating_sub(bot.health.min(100)) as f32 * 0.5;
        let (cleanup_priority, cleanup_readiness_penalty) =
            if completed_structure_owners.contains(&bot.owner) {
                (140.0, pressure_readiness_penalty)
            } else {
                (500.0, 0.0)
            };
        let score = economy.value
            + cleanup_priority
            + danger
            + vulnerability
            + forces.friendly_strength * 0.04
            - cleanup_readiness_penalty
            - distance * 2.0
            - route_exposure(home, bot.position, state.terrain, budget) * 14.0;
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

fn estimate_forces(
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
            bot.charge.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let strength = bot.health as f32 * (0.35 + charge * 0.65);
        if bot.owner == owner {
            estimate.friendly_defenders += 1;
            estimate.friendly_strength += strength;
            estimate.low_charge_defenders += usize::from(charge < 0.45);
        } else {
            estimate.enemy_strength += strength;
        }
    }
    estimate
}

fn deposit_value(
    deposit: &DepositState,
    home: Vec2,
    state: &GameState<'_>,
    budget: &mut WorkBudget,
) -> f32 {
    let travel = home.distance(deposit.position) / crate::ZONE_BLOCK_SIZE;
    let exposure = route_exposure(home, deposit.position, state.terrain, budget);
    (deposit.amount as f32 + 1.0).ln() * 10.0 - travel * 6.0 - exposure * 10.0
}

fn materialize_plan(
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

fn push_desired(
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

fn intent_edits(
    owner: SwarmId,
    grid: &IntentGrid,
    desired: &[DesiredIntent],
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
                paint: true,
            });
        }
    }
    for (cell, existing) in grid.iter_active_cells().take(MAX_ACTIVE_CELLS) {
        for kind in IntentKind::ALL {
            if !budget.spend(1) {
                break;
            }
            if existing.has_owned(kind, owner) && !desired.contains(&DesiredIntent { cell, kind }) {
                edits.push(IntentEdit {
                    cell,
                    kind,
                    paint: false,
                });
            }
        }
    }
    edits.sort_by_key(|edit| (!edit.paint, edit.cell.y, edit.cell.x, edit.kind.index()));
    edits
}

fn route_exposure(start: Vec2, end: Vec2, terrain: &[Obstacle], budget: &mut WorkBudget) -> f32 {
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

fn world_to_intent_cell(position: Vec2) -> IVec2 {
    (position / crate::ZONE_BLOCK_SIZE).floor().as_ivec2()
}

fn intent_cell_center(cell: IVec2) -> Vec2 {
    (cell.as_vec2() + Vec2::splat(0.5)) * crate::ZONE_BLOCK_SIZE
}

fn midpoint_cell(left: IVec2, right: IVec2) -> IVec2 {
    IVec2::new((left.x + right.x) / 2, (left.y + right.y) / 2)
}

fn line_cells(start: IVec2, target: IVec2, limit: usize) -> Vec<IVec2> {
    let mut cells = Vec::new();
    let mut current = start;
    cells.push(current);
    while current != target && cells.len() < limit {
        current += (target - current).signum();
        cells.push(current);
    }
    cells
}

impl TimedController {
    fn decide(&mut self) -> Decision {
        if self.ticks_until_advance > 0 {
            self.ticks_until_advance -= 1;
            return Decision::idle("timed controller waiting for next advance");
        }

        let next = self.assault_cell + (self.target_cell - self.assault_cell).signum();
        if next == self.assault_cell {
            return Decision::idle("timed controller has reached its target");
        }

        let previous = self.assault_cell;
        self.assault_cell = next;
        self.ticks_until_advance = self.period;
        Decision {
            edits: vec![
                IntentEdit {
                    cell: next,
                    kind: IntentKind::Defend,
                    paint: true,
                },
                IntentEdit {
                    cell: previous,
                    kind: IntentKind::Defend,
                    paint: false,
                },
            ],
            explanation: format!("timed controller advanced Defend intent to {next}"),
            reviewed: true,
            work_units: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(cell: IVec2) -> Vec2 {
        (cell.as_vec2() + Vec2::splat(0.5)) * crate::ZONE_BLOCK_SIZE
    }

    fn apply(grid: &mut IntentGrid, owner: SwarmId, decision: &Decision) {
        for edit in &decision.edits {
            if edit.paint {
                grid.paint(edit.cell, edit.kind, owner);
            } else {
                grid.erase(edit.cell, edit.kind, owner);
            }
        }
    }

    fn empty_state<'a>(grid: &'a IntentGrid, tick: u64) -> GameState<'a> {
        GameState {
            grid,
            swarms: &[],
            bots: &[],
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick,
            finished: false,
        }
    }

    fn mature_economy_bots(owner: SwarmId, home: Vec2) -> Vec<BotState> {
        (0..4)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Worker,
                position: home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .chain((4..8).map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Hauler,
                position: home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            }))
            .collect()
    }

    fn funded_primary_chain(
        owner: SwarmId,
        home: Vec2,
        primary: DepositState,
    ) -> [StructureState; 2] {
        [
            StructureState {
                id: 30,
                owner,
                position: primary.position + Vec2::new(96.0, 0.0),
                kind: StructureKind::Source,
                health: 100,
                minerals: 40,
            },
            StructureState {
                id: 31,
                owner,
                position: home,
                kind: StructureKind::Facility,
                health: 100,
                minerals: 20,
            },
        ]
    }

    #[test]
    fn timed_controller_preserves_frozen_advance_schedule_and_edits() {
        let grid = IntentGrid::new(16, 16);
        let mut controller = Controller::timed(SwarmId(7), IVec2::new(3, 3), IVec2::ZERO, 2, 2);

        assert!(controller.decide(&empty_state(&grid, 0)).edits.is_empty());
        assert!(controller.decide(&empty_state(&grid, 1)).edits.is_empty());
        let first = controller.decide(&empty_state(&grid, 2));
        assert_eq!(
            first.edits,
            vec![
                IntentEdit {
                    cell: IVec2::new(2, 2),
                    kind: IntentKind::Defend,
                    paint: true,
                },
                IntentEdit {
                    cell: IVec2::new(3, 3),
                    kind: IntentKind::Defend,
                    paint: false,
                },
            ]
        );
        assert!(controller.decide(&empty_state(&grid, 3)).edits.is_empty());
        assert!(controller.decide(&empty_state(&grid, 4)).edits.is_empty());
        assert_eq!(
            controller.decide(&empty_state(&grid, 5)).edits[0].cell,
            IVec2::new(1, 1)
        );
    }

    #[test]
    fn timed_controller_is_quiet_after_reaching_its_target() {
        let grid = IntentGrid::new(16, 16);
        let mut controller = Controller::timed(SwarmId(7), IVec2::new(1, 0), IVec2::ZERO, 0, 1);
        assert!(controller.decide(&empty_state(&grid, 0)).reviewed);
        assert!(!controller.decide(&empty_state(&grid, 1)).reviewed);

        let reached = controller.decide(&empty_state(&grid, 2));
        assert!(!reached.reviewed);
        assert!(reached.edits.is_empty());
        assert_eq!(reached.work_units, 0);
    }

    #[test]
    fn adaptive_controller_coordinates_economy_support_and_combat_on_first_review() {
        let grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 40,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(8, 0)),
                minerals: 20,
            },
        ];
        let bots = [
            BotState {
                id: 1,
                owner,
                kind: NanobotType::Worker,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            },
            BotState {
                id: 2,
                owner,
                kind: NanobotType::Hauler,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            },
            BotState {
                id: 3,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 0.3,
                cargo: 0,
            },
        ];
        let structures = [
            StructureState {
                id: 10,
                owner,
                position: swarms[0].home,
                kind: StructureKind::Facility,
                health: 100,
                minerals: 0,
            },
            StructureState {
                id: 11,
                owner: enemy,
                position: world(IVec2::new(7, 0)),
                kind: StructureKind::Sink,
                health: 100,
                minerals: 80,
            },
        ];
        let deposits = [DepositState {
            id: 20,
            position: world(IVec2::new(2, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let state = GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        };

        let decision = Controller::adaptive(owner).decide(&state);

        assert!(decision.reviewed);
        assert!(decision.work_units <= PLANNING_WORK_BUDGET);
        assert!(decision.edits.iter().all(|edit| edit.paint));
        assert!(decision.edits.iter().all(|edit| grid.in_bounds(edit.cell)));
        assert!(
            decision
                .edits
                .iter()
                .any(|edit| { edit.kind == IntentKind::Gather && edit.cell == IVec2::new(2, 0) })
        );
        for kind in IntentKind::ALL {
            assert!(
                decision.edits.iter().any(|edit| edit.kind == kind),
                "coordinated plan omitted {kind:?}"
            );
        }
        let defend_tiles = decision
            .edits
            .iter()
            .filter(|edit| edit.kind == IntentKind::Defend)
            .count();
        assert!(
            defend_tiles >= 4,
            "plan needs broad enough territory to create Defender demand"
        );
    }

    #[test]
    fn raid_retains_the_established_primary() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let home = world(IVec2::ZERO);
        let initial_swarms = [SwarmState {
            id: owner,
            home,
            minerals: 0,
        }];
        let economy_bots = mature_economy_bots(owner, home);
        let deposits = [
            DepositState {
                id: 20,
                position: world(IVec2::new(-1, 0)),
                amount: 72_000,
                radius: 64.0,
            },
            DepositState {
                id: 21,
                position: world(IVec2::new(4, 0)),
                amount: 72_000,
                radius: 64.0,
            },
        ];
        let primary_chain = funded_primary_chain(owner, home, deposits[0]);
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &initial_swarms,
            bots: &economy_bots,
            structures: &primary_chain,
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(IVec2::new(-1, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
        );
        assert!(
            !grid
                .cell(IVec2::new(4, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
        );

        let supplied_swarms = [
            SwarmState {
                minerals: 2_000,
                ..initial_swarms[0]
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(8, 0)),
                minerals: 0,
            },
        ];
        let defenders = (0..8).map(|id| BotState {
            id: 100 + id,
            owner,
            kind: NanobotType::Defender,
            position: home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let bots = economy_bots
            .into_iter()
            .chain(defenders)
            .collect::<Vec<_>>();
        let enemy_sink = StructureState {
            id: 40,
            owner: enemy,
            position: supplied_swarms[1].home,
            kind: StructureKind::Sink,
            health: 20,
            minerals: 100,
        };
        let depleted_primary_chain = [
            StructureState {
                minerals: 0,
                ..primary_chain[0]
            },
            StructureState {
                minerals: 0,
                ..primary_chain[1]
            },
        ];
        let raid = controller.decide(&GameState {
            grid: &grid,
            swarms: &supplied_swarms,
            bots: &bots,
            structures: &[
                depleted_primary_chain[0],
                depleted_primary_chain[1],
                enemy_sink,
            ],
            deposits: &deposits,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });

        assert!(raid.explanation.contains("attack enemy logistics"));
        assert!(!raid.edits.iter().any(|edit| {
            !edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(-1, 0)
        }));
    }

    #[test]
    fn advantaged_raid_paints_enemy_logistics_and_keeps_home_economy() {
        let grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 1_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(8, 0)),
                minerals: 0,
            },
        ];
        let bots = (0..8)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let deposits = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let structures = [StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(7, 0)),
            kind: StructureKind::Sink,
            health: 20,
            minerals: 80,
        }];

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });

        assert!(decision.explanation.contains("attack enemy logistics"));
        assert!(decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(-1, 0)
        }));
        assert!(decision.edits.iter().any(|edit| {
            edit.paint
                && edit.kind == IntentKind::Build
                && edit.cell != IVec2::ZERO
                && edit.cell.abs().max_element() <= 1
        }));
        assert!(decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(7, 0)
        }));
    }

    #[test]
    fn progressing_nanobot_front_is_not_abandoned_for_fresh_targets() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let friendly_defenders = (0..2)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let rich_charger = StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(6, 2)),
            kind: StructureKind::Charger,
            health: 1,
            minerals: 1_000,
        };
        let damage_target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(IVec2::new(6, -2)),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let mut bots = friendly_defenders.clone();
        bots.push(damage_target);
        let mut controller = Controller::adaptive(owner);

        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);

        assert!(initial.explanation.contains("Nanobot 50"));
        assert!(
            grid.cell(IVec2::new(6, -2))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        let progressing_target = BotState {
            health: 20,
            ..damage_target
        };
        let fresh_target = BotState {
            id: 51,
            position: world(IVec2::new(4, 4)),
            ..damage_target
        };
        bots.truncate(friendly_defenders.len());
        bots.extend([progressing_target, fresh_target]);
        let revised = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[rich_charger],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        apply(&mut grid, owner, &revised);

        assert!(
            grid.cell(IVec2::new(6, -2))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            !grid
                .cell(IVec2::new(4, 4))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            !grid
                .cell(IVec2::new(6, 2))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            revised
                .explanation
                .contains("retained hunt remaining Nanobot 50")
        );
    }

    #[test]
    fn progressing_live_attack_remains_when_a_supported_new_target_appears() {
        let mut grid = IntentGrid::new(64, 64);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(24, 24)),
                minerals: 0,
            },
        ];
        let bots = (0..10)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let friendly_charger = StructureState {
            id: 20,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 100,
        };
        let incumbent = StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(24, 24)),
            kind: StructureKind::Sink,
            health: 100,
            minerals: 0,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, incumbent],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(IVec2::new(24, 24))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );

        let progressing_incumbent = StructureState {
            health: 80,
            ..incumbent
        };
        let new_forward_target = StructureState {
            id: 31,
            owner: enemy,
            position: world(IVec2::new(10, 10)),
            kind: StructureKind::Charger,
            health: 5,
            minerals: 1_000,
        };
        let revised = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, progressing_incumbent, new_forward_target],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        apply(&mut grid, owner, &revised);

        assert!(
            grid.cell(IVec2::new(24, 24))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            grid.cell(IVec2::new(10, 10))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            grid.iter_active_cells()
                .filter(|(_, intent)| IntentKind::ALL
                    .into_iter()
                    .any(|kind| intent.has_owned(kind, owner)))
                .count()
                <= MAX_PLAN_CELLS
        );

        let depleted_defenders = bots
            .iter()
            .map(|bot| BotState {
                charge: 0.1,
                ..*bot
            })
            .collect::<Vec<_>>();
        let empty_charger = StructureState {
            minerals: 0,
            ..friendly_charger
        };
        let unsupported = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &depleted_defenders,
            structures: &[empty_charger, progressing_incumbent, new_forward_target],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 2,
            finished: false,
        });
        apply(&mut grid, owner, &unsupported);

        let retained_fronts = [IVec2::new(24, 24), IVec2::new(10, 10)]
            .into_iter()
            .filter(|cell| {
                grid.cell(*cell)
                    .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
            })
            .count();
        assert_eq!(retained_fronts, 1);
    }

    #[test]
    fn supported_attack_targets_a_recovery_bot_while_enemy_structures_remain() {
        let grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let mut bots = (0..8)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        bots.push(BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(IVec2::new(3, 5)),
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let structures = [
            StructureState {
                id: 20,
                owner,
                position: world(IVec2::new(1, 0)),
                kind: StructureKind::Charger,
                health: 100,
                minerals: 100,
            },
            StructureState {
                id: 30,
                owner: enemy,
                position: world(IVec2::new(12, 0)),
                kind: StructureKind::Sink,
                health: 100,
                minerals: 0,
            },
        ];

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });

        assert!(decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(12, 0)
        }));
        assert!(decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(3, 5)
        }));
    }

    #[test]
    fn progressing_incumbent_wins_redeployment_when_only_one_front_is_supportable() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let bots = (0..2)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let friendly_charger = StructureState {
            id: 20,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 100,
        };
        let incumbent = StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(12, 0)),
            kind: StructureKind::Sink,
            health: 100,
            minerals: 0,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, incumbent],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);

        let progressing_incumbent = StructureState {
            health: 20,
            ..incumbent
        };
        let new_forward_target = StructureState {
            id: 31,
            owner: enemy,
            position: world(IVec2::new(3, 5)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 1_000,
        };
        let revised = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, progressing_incumbent, new_forward_target],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        apply(&mut grid, owner, &revised);

        assert!(
            revised
                .explanation
                .contains("retained attack enemy logistics structure 30"),
            "{}",
            revised.explanation
        );
        assert!(!revised.explanation.contains("structure 31"));
        assert!(
            grid.cell(IVec2::new(12, 0))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            !grid
                .cell(IVec2::new(3, 5))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
    }

    #[test]
    fn depleted_target_triggers_an_urgent_switch_to_a_useful_deposit() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 0,
        }];
        let initial_deposits = [
            DepositState {
                id: 20,
                position: world(IVec2::new(2, 0)),
                amount: 1_000,
                radius: 100.0,
            },
            DepositState {
                id: 21,
                position: world(IVec2::new(4, 0)),
                amount: 50,
                radius: 100.0,
            },
        ];
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &initial_deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(initial.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(2, 0)
        }));
        apply(&mut grid, owner, &initial);

        let changed_deposits = [
            DepositState {
                amount: 0,
                ..initial_deposits[0]
            },
            initial_deposits[1],
        ];
        let urgent = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &changed_deposits,
            terrain: &[],
            tick: 1,
            finished: false,
        });

        assert!(urgent.reviewed, "depletion must bypass the regular cadence");
        assert!(urgent.explanation.contains("exhausted"));
        assert!(urgent.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(4, 0)
        }));
        assert!(urgent.edits.iter().any(|edit| {
            !edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(2, 0)
        }));
        assert!(urgent.work_units <= PLANNING_WORK_BUDGET);
    }

    #[test]
    fn viable_plan_is_quiet_between_reviews_and_retained_at_regular_review() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 20,
        }];
        let deposits = [DepositState {
            id: 20,
            position: world(IVec2::new(2, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);

        let between = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS - 1,
            finished: false,
        });
        assert!(!between.reviewed);
        assert!(between.edits.is_empty());

        let regular = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(regular.reviewed);
        assert!(regular.edits.is_empty());
        assert!(regular.explanation.contains("retained viable"));
        assert!(regular.work_units <= PLANNING_WORK_BUDGET);
    }

    #[test]
    fn clearly_better_resource_opportunity_replaces_a_viable_plan() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 0,
        }];
        let original = DepositState {
            id: 20,
            position: world(IVec2::new(2, 0)),
            amount: 100,
            radius: 100.0,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &[original],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);

        let opportunities = [
            original,
            DepositState {
                id: 21,
                position: world(IVec2::new(4, 0)),
                amount: 1_000_000,
                radius: 100.0,
            },
        ];
        let changed = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &opportunities,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });

        assert!(changed.reviewed);
        assert!(changed.explanation.contains("clearly better"));
        assert!(changed.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(4, 0)
        }));
        assert!(changed.edits.iter().any(|edit| {
            !edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(2, 0)
        }));
    }

    #[test]
    fn accumulated_supply_and_a_logistics_target_replace_the_economy_plan() {
        let mut grid = IntentGrid::new(64, 64);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let initial_swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 0,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(24, 24)),
                minerals: 0,
            },
        ];
        let bots = [owner, enemy]
            .into_iter()
            .flat_map(|bot_owner| {
                (0..3).map(move |id| BotState {
                    id: bot_owner.0 as u64 * 10 + id,
                    owner: bot_owner,
                    kind: NanobotType::Defender,
                    position: if bot_owner == owner {
                        initial_swarms[0].home
                    } else {
                        initial_swarms[1].home
                    },
                    health: 100,
                    charge: 1.0,
                    cargo: 0,
                })
            })
            .collect::<Vec<_>>();
        let deposit = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, -1)),
            amount: 72_000,
            radius: 64.0,
        }];
        let facility = StructureState {
            id: 30,
            owner: enemy,
            position: initial_swarms[1].home,
            kind: StructureKind::Facility,
            health: 100,
            minerals: 0,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &initial_swarms,
            bots: &bots,
            structures: &[facility],
            deposits: &deposit,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(initial.explanation.contains("secure Resource Deposit"));
        apply(&mut grid, owner, &initial);

        let supplied_swarms = [
            SwarmState {
                minerals: 2_000,
                ..initial_swarms[0]
            },
            initial_swarms[1],
        ];
        let sink = StructureState {
            id: 31,
            kind: StructureKind::Sink,
            minerals: 100,
            ..facility
        };
        let pressure = controller.decide(&GameState {
            grid: &grid,
            swarms: &supplied_swarms,
            bots: &bots,
            structures: &[facility, sink],
            deposits: &deposit,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });

        assert!(pressure.explanation.contains("attack enemy logistics"));
        assert!(pressure.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(24, 24)
        }));
        assert!(!pressure.edits.iter().any(|edit| {
            !edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(-1, -1)
        }));
    }

    #[test]
    fn losing_a_key_charger_triggers_an_urgent_review() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 20,
        }];
        let deposits = [DepositState {
            id: 20,
            position: world(IVec2::new(2, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let charger = [StructureState {
            id: 42,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 25,
        }];
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &charger,
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(initial.edits.iter().any(|edit| {
            edit.paint && edit.cell == IVec2::new(1, 0) && edit.kind == IntentKind::Defend
        }));
        assert!(initial.edits.iter().any(|edit| {
            edit.paint && edit.cell == IVec2::new(1, 0) && edit.kind == IntentKind::Corridor
        }));
        apply(&mut grid, owner, &initial);

        let urgent = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &[],
            tick: 1,
            finished: false,
        });

        assert!(urgent.reviewed);
        assert!(urgent.explanation.contains("key Charger 42 was lost"));
        assert!(urgent.work_units <= PLANNING_WORK_BUDGET);
    }

    #[test]
    fn terrain_exposure_changes_which_equal_deposit_is_secured() {
        let grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 0,
        }];
        let exposed = DepositState {
            id: 20,
            position: world(IVec2::new(4, 0)),
            amount: 500,
            radius: 100.0,
        };
        let clear = DepositState {
            id: 21,
            position: world(IVec2::new(0, 4)),
            amount: 500,
            radius: 100.0,
        };
        let deposits = [clear, exposed];
        let terrain = [Obstacle::Circle {
            center: world(IVec2::new(2, 0)),
            radius: crate::ZONE_BLOCK_SIZE * 0.45,
        }];

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposits,
            terrain: &terrain,
            tick: 0,
            finished: false,
        });

        assert!(decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(0, 4)
        }));
        assert!(!decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(4, 0)
        }));
    }

    #[test]
    fn irrelevant_far_rocks_do_not_dilute_an_obstructed_deposit_approach() {
        let grid = IntentGrid::new(64, 64);
        let owner = SwarmId(4);
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 0,
        }];
        let obstructed = DepositState {
            id: 20,
            position: world(IVec2::new(4, 0)),
            amount: 500,
            radius: 100.0,
        };
        let clear = DepositState {
            id: 21,
            position: world(IVec2::new(0, 4)) + Vec2::new(0.0, 10.0),
            amount: 500,
            radius: 100.0,
        };
        let deposits = [clear, obstructed];
        let approach_rock = Obstacle::Circle {
            center: world(IVec2::new(2, 0)),
            radius: crate::ZONE_BLOCK_SIZE * 0.45,
        };
        let mut terrain = vec![approach_rock];
        terrain.extend((0..200).map(|index| Obstacle::Circle {
            center: Vec2::new(50_000.0 + index as f32 * 100.0, 50_000.0),
            radius: 10.0,
        }));

        for terrain in [&[approach_rock][..], terrain.as_slice()] {
            let decision = Controller::adaptive(owner).decide(&GameState {
                grid: &grid,
                swarms: &swarms,
                bots: &[],
                structures: &[],
                deposits: &deposits,
                terrain,
                tick: 0,
                finished: false,
            });
            assert!(
                decision.edits.iter().any(|edit| {
                    edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(0, 4)
                }),
                "the clear approach lost preference after {} irrelevant rocks",
                terrain.len().saturating_sub(1)
            );
            assert!(!decision.edits.iter().any(|edit| {
                edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(4, 0)
            }));
        }
    }

    #[test]
    fn combat_support_build_follows_the_living_defender_cohort_without_outlier_drift() {
        let mut grid = IntentGrid::new(64, 64);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let home_cell = IVec2::ZERO;
        let cohort_cell = IVec2::new(4, 0);
        let midpoint = IVec2::new(6, 0);
        let target_cell = IVec2::new(12, 0);
        let home = world(home_cell);
        let swarms = [
            SwarmState {
                id: owner,
                home,
                minerals: 1_000,
            },
            SwarmState {
                id: enemy,
                home: world(target_cell),
                minerals: 0,
            },
        ];
        let mut bots = (0..5)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        bots.extend([
            BotState {
                id: 5,
                owner,
                kind: NanobotType::Defender,
                position: world(IVec2::new(11, 0)),
                health: 100,
                charge: 1.0,
                cargo: 0,
            },
            BotState {
                id: 6,
                owner,
                kind: NanobotType::Worker,
                position: world(IVec2::new(10, 0)),
                health: 100,
                charge: 1.0,
                cargo: 0,
            },
            BotState {
                id: 7,
                owner: enemy,
                kind: NanobotType::Defender,
                position: world(IVec2::new(-10, 0)),
                health: 100,
                charge: 1.0,
                cargo: 0,
            },
            BotState {
                id: 8,
                owner,
                kind: NanobotType::Defender,
                position: world(IVec2::new(10, 0)),
                health: 0,
                charge: 1.0,
                cargo: 0,
            },
        ]);
        let deposit = [DepositState {
            id: 20,
            position: world(IVec2::new(-2, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let structures = [
            StructureState {
                id: 29,
                owner,
                position: world(IVec2::new(-1, 0)),
                kind: StructureKind::Charger,
                health: 100,
                minerals: 0,
            },
            StructureState {
                id: 30,
                owner: enemy,
                position: world(target_cell),
                kind: StructureKind::Sink,
                health: 20,
                minerals: 100,
            },
        ];
        let mut controller = Controller::adaptive(owner);

        let home_decision = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposit,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(
            home_decision
                .explanation
                .contains("attack enemy logistics structure 30")
        );
        assert!(home_decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(-2, 0)
        }));
        assert!(home_decision.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == target_cell
        }));
        assert!(
            home_decision
                .edits
                .iter()
                .filter(|edit| {
                    edit.paint && edit.kind == IntentKind::Build && edit.cell != home_cell
                })
                .all(|edit| (edit.cell - home_cell).abs().max_element() <= 1),
            "support must stay with the home cohort rather than open at midpoint {midpoint}: {:?}",
            home_decision.edits
        );
        assert!(
            !home_decision.edits.iter().any(|edit| {
                edit.paint && edit.kind == IntentKind::Build && edit.cell == midpoint
            })
        );
        apply(&mut grid, owner, &home_decision);

        for bot in bots.iter_mut().filter(|bot| bot.id < 5) {
            bot.position = world(cohort_cell);
        }
        let obstruction = Obstacle::Circle {
            center: world(cohort_cell),
            radius: crate::ZONE_BLOCK_SIZE * 0.6,
        };
        let advanced = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposit,
            terrain: &[obstruction],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        apply(&mut grid, owner, &advanced);

        let forward_builds = grid
            .iter_active_cells()
            .filter_map(|(cell, intent)| {
                (cell != home_cell && intent.has_owned(IntentKind::Build, owner)).then_some(cell)
            })
            .collect::<Vec<_>>();
        assert!(
            forward_builds.iter().any(|cell| {
                (*cell - cohort_cell).abs().max_element() <= 1
                    && obstruction.admits_body(world(*cell))
            }),
            "an open support Build must follow the advanced cohort: {forward_builds:?}; {}",
            advanced.explanation
        );
        assert!(
            forward_builds
                .iter()
                .all(|cell| (*cell - cohort_cell).abs().max_element() <= 1),
            "outliers or remote obstacle alternatives dragged support away: {forward_builds:?}"
        );
        assert!(!forward_builds.contains(&midpoint));
        assert!(
            grid.cell(target_cell)
                .is_some_and(|intent| { intent.has_owned(IntentKind::Defend, owner) })
        );
        assert!(
            grid.cell(IVec2::new(-2, 0))
                .is_some_and(|intent| { intent.has_owned(IntentKind::Gather, owner) })
        );
    }

    #[test]
    fn combat_support_retains_a_nearby_open_build_during_small_cohort_drift() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let home_cell = IVec2::ZERO;
        let home = world(home_cell);
        let swarms = [
            SwarmState {
                id: owner,
                home,
                minerals: 1_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(8, 0)),
                minerals: 0,
            },
        ];
        let mut bots = (0..6)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let deposits = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let structures = [StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(8, 0)),
            kind: StructureKind::Sink,
            health: 20,
            minerals: 100,
        }];
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(
            initial
                .explanation
                .contains("attack enemy logistics structure 30")
        );
        apply(&mut grid, owner, &initial);
        let incumbent = grid
            .iter_active_cells()
            .find_map(|(cell, intent)| {
                (cell != home_cell && intent.has_owned(IntentKind::Build, owner)).then_some(cell)
            })
            .expect("the attack plan needs one non-home support Build");

        for bot in &mut bots {
            bot.position = world(IVec2::new(1, 0));
        }
        let drift = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(
            !drift.edits.iter().any(|edit| {
                edit.kind == IntentKind::Build && (edit.cell == incumbent || edit.paint)
            }),
            "an open support Build one cell from the cohort must not churn: {:?}",
            drift.edits
        );
        apply(&mut grid, owner, &drift);
        assert!(
            grid.cell(incumbent)
                .is_some_and(|intent| intent.has_owned(IntentKind::Build, owner))
        );

        let obstruction = Obstacle::Circle {
            center: world(incumbent),
            radius: crate::ZONE_BLOCK_SIZE * 0.6,
        };
        let obstructed = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[obstruction],
            tick: REVIEW_PERIOD_TICKS * 2,
            finished: false,
        });
        apply(&mut grid, owner, &obstructed);
        assert!(
            grid.cell(incumbent)
                .is_none_or(|intent| !intent.has_owned(IntentKind::Build, owner)),
            "an obstructed incumbent must be released"
        );
        assert!(grid.iter_active_cells().any(|(cell, intent)| {
            cell != home_cell
                && (cell - IVec2::new(1, 0)).abs().max_element() <= 1
                && obstruction.admits_body(world(cell))
                && intent.has_owned(IntentKind::Build, owner)
        }));
    }

    #[test]
    fn even_split_cohort_support_uses_an_occupied_flank_instead_of_the_empty_median_gap() {
        let grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let home = world(IVec2::ZERO);
        let swarms = [
            SwarmState {
                id: owner,
                home,
                minerals: 1_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let bots = (0..6)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: world(if id < 3 {
                    IVec2::ZERO
                } else {
                    IVec2::new(6, 0)
                }),
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let deposits = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let structures = [StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(12, 0)),
            kind: StructureKind::Sink,
            health: 20,
            minerals: 100,
        }];

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(
            decision
                .explanation
                .contains("attack enemy logistics structure 30")
        );
        let forward_builds = decision
            .edits
            .iter()
            .filter(|edit| edit.paint && edit.kind == IntentKind::Build && edit.cell != IVec2::ZERO)
            .map(|edit| edit.cell)
            .collect::<Vec<_>>();
        assert!(
            forward_builds
                .iter()
                .all(|cell| cell.abs().max_element() <= 1),
            "equal cohort flanks must choose the home-nearest occupied flank, not the empty median gap: {forward_builds:?}"
        );
        assert!(
            forward_builds
                .iter()
                .all(|cell| { (*cell - IVec2::new(3, 0)).abs().max_element() > 1 })
        );
    }

    #[test]
    fn pressure_support_uses_the_open_side_of_a_swapped_obstruction() {
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let home = world(IVec2::ZERO);
        let cohort_cell = IVec2::new(4, 0);
        let enemy_home = world(IVec2::new(12, 0));
        let swarms = [
            SwarmState {
                id: owner,
                home,
                minerals: 1_000,
            },
            SwarmState {
                id: enemy,
                home: enemy_home,
                minerals: 0,
            },
        ];
        let bots = [
            Vec2::new(-250.0, -250.0),
            Vec2::new(250.0, -250.0),
            Vec2::new(-250.0, 250.0),
            Vec2::new(250.0, 250.0),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, offset)| BotState {
            id: id as u64,
            owner,
            kind: NanobotType::Defender,
            position: world(cohort_cell) + offset,
            health: 100,
            charge: 1.0,
            cargo: 0,
        })
        .collect::<Vec<_>>();
        let deposit = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let enemy_sink = [StructureState {
            id: 30,
            owner: enemy,
            position: enemy_home,
            kind: StructureKind::Sink,
            health: 20,
            minerals: 100,
        }];

        for (blocked_side, expected_open_side) in [(1, -1), (-1, 1)] {
            let grid = IntentGrid::new(64, 64);
            let terrain = [
                Obstacle::Circle {
                    center: world(cohort_cell),
                    radius: crate::ZONE_BLOCK_SIZE * 0.6,
                },
                Obstacle::Circle {
                    center: world(cohort_cell + IVec2::new(0, blocked_side)),
                    radius: crate::ZONE_BLOCK_SIZE * 0.6,
                },
            ];
            assert!(bots.iter().all(|bot| {
                terrain
                    .iter()
                    .all(|obstacle| obstacle.admits_body(bot.position))
            }));
            let decision = Controller::adaptive(owner).decide(&GameState {
                grid: &grid,
                swarms: &swarms,
                bots: &bots,
                structures: &enemy_sink,
                deposits: &deposit,
                terrain: &terrain,
                tick: 0,
                finished: false,
            });

            assert!(
                decision.edits.iter().any(|edit| {
                    edit.paint
                        && edit.kind == IntentKind::Build
                        && edit.cell.x == cohort_cell.x
                        && (edit.cell - cohort_cell).abs().max_element() == 1
                        && edit.cell.y.signum() == expected_open_side
                }),
                "expected open side {expected_open_side}: {}; edits: {:?}",
                decision.explanation,
                decision.edits
            );
            assert!(decision.edits.iter().any(|edit| {
                edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(12, 0)
            }));
            assert!(decision.edits.iter().any(|edit| {
                edit.paint && edit.kind == IntentKind::Gather && edit.cell == IVec2::new(-1, 0)
            }));
        }
    }

    #[test]
    fn adaptive_review_stays_inside_the_hard_work_budget() {
        let owner = SwarmId(4);
        let mut grid = IntentGrid::new(64, 64);
        for kind in IntentKind::ALL {
            grid.paint(IVec2::ZERO, kind, owner);
        }
        let swarms = [SwarmState {
            id: owner,
            home: world(IVec2::ZERO),
            minerals: 0,
        }];
        let bots = (0..1_000)
            .map(|id| BotState {
                id,
                owner: if id % 2 == 0 { owner } else { SwarmId(9) },
                kind: NanobotType::Defender,
                position: Vec2::ZERO,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let deposits = (0..1_000)
            .map(|id| DepositState {
                id,
                position: world(IVec2::new((id % 20) as i32, (id / 20 % 20) as i32)),
                amount: 100,
                radius: 100.0,
            })
            .collect::<Vec<_>>();
        let terrain = (0..1_000)
            .map(|id| Obstacle::Circle {
                center: Vec2::splat(50_000.0 + id as f32),
                radius: 10.0,
            })
            .collect::<Vec<_>>();

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &deposits,
            terrain: &terrain,
            tick: 0,
            finished: false,
        });

        assert!(decision.reviewed);
        assert!(
            decision
                .explanation
                .contains("planning allowance exhausted")
        );
        assert!(decision.work_units <= PLANNING_WORK_BUDGET);
        assert!(decision.edits.iter().all(|edit| grid.in_bounds(edit.cell)));
        assert!(
            decision.edits.iter().all(|edit| edit.paint),
            "an incomplete over-budget candidate must not erase existing intent"
        );
    }

    #[test]
    fn authored_terrain_leaves_budget_for_a_coordinated_plan() {
        let grid = IntentGrid::new(64, 64);
        let owner = SwarmId::PLAYER;
        let enemy = SwarmId(1);
        let swarms = [
            SwarmState {
                id: owner,
                home: crate::scenario::cell_origin(crate::scenario::PLAYER_CELL),
                minerals: 0,
            },
            SwarmState {
                id: enemy,
                home: crate::scenario::cell_origin(crate::scenario::OPPONENT_CELL),
                minerals: 0,
            },
        ];
        let bots = NanobotType::ALL
            .into_iter()
            .flat_map(|kind| {
                (0..3).map(move |id| BotState {
                    id: kind as u64 * 3 + id,
                    owner,
                    kind,
                    position: swarms[0].home,
                    health: 100,
                    charge: 1.0,
                    cargo: 0,
                })
            })
            .collect::<Vec<_>>();
        let deposit_cells = [
            crate::scenario::PLAYER_DEPOSIT_CELL,
            crate::scenario::OPPONENT_DEPOSIT_CELL,
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[0],
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[1],
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[2],
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[3],
        ];
        let deposits = deposit_cells
            .into_iter()
            .enumerate()
            .map(|(id, cell)| DepositState {
                id: id as u64,
                position: crate::scenario::cell_origin(cell),
                amount: crate::scenario::STARTING_DEPOSIT_AMOUNT,
                radius: crate::scenario::STARTING_WORK_RADIUS,
            })
            .collect::<Vec<_>>();
        let terrain = crate::scenario::default_rock_geometry()
            .into_iter()
            .map(|(rock, transform)| rock.obstacle(&transform))
            .collect::<Vec<_>>();

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &deposits,
            terrain: &terrain,
            tick: 0,
            finished: false,
        });

        assert!(decision.work_units <= PLANNING_WORK_BUDGET);
        for kind in IntentKind::ALL {
            assert!(
                decision
                    .edits
                    .iter()
                    .any(|edit| edit.paint && edit.kind == kind),
                "authored terrain plan omitted {kind:?} at {} work units with {} rocks",
                decision.work_units,
                terrain.len()
            );
        }
    }

    #[test]
    fn authored_terrain_advantaged_raid_reaches_target_and_preserves_gather() {
        let grid = IntentGrid::new(64, 64);
        let owner = SwarmId::PLAYER;
        let enemy = SwarmId(1);
        let swarms = [
            SwarmState {
                id: owner,
                home: crate::scenario::cell_origin(crate::scenario::PLAYER_CELL),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: crate::scenario::cell_origin(crate::scenario::OPPONENT_CELL),
                minerals: 0,
            },
        ];
        let bots = (0..10)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let deposit_cells = [
            crate::scenario::PLAYER_DEPOSIT_CELL,
            crate::scenario::OPPONENT_DEPOSIT_CELL,
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[0],
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[1],
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[2],
            crate::scenario::NEUTRAL_DEPOSIT_CELLS[3],
        ];
        let deposits = deposit_cells
            .into_iter()
            .enumerate()
            .map(|(id, cell)| DepositState {
                id: 20 + id as u64,
                position: crate::scenario::cell_origin(cell),
                amount: crate::scenario::STARTING_DEPOSIT_AMOUNT,
                radius: crate::scenario::STARTING_WORK_RADIUS,
            })
            .collect::<Vec<_>>();
        let structures = [StructureState {
            id: 30,
            owner: enemy,
            position: swarms[1].home,
            kind: StructureKind::Sink,
            health: 20,
            minerals: 100,
        }];
        let terrain = crate::scenario::default_rock_geometry()
            .into_iter()
            .map(|(rock, transform)| rock.obstacle(&transform))
            .collect::<Vec<_>>();

        let decision = Controller::adaptive(owner).decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &structures,
            deposits: &deposits,
            terrain: &terrain,
            tick: 0,
            finished: false,
        });

        assert!(decision.explanation.contains("attack enemy logistics"));
        assert!(decision.edits.iter().any(|edit| {
            edit.paint
                && edit.kind == IntentKind::Gather
                && edit.cell == crate::scenario::PLAYER_DEPOSIT_CELL
        }));
        assert!(decision.edits.iter().any(|edit| {
            edit.paint
                && edit.kind == IntentKind::Defend
                && edit.cell == crate::scenario::OPPONENT_CELL
        }));
        assert!(decision.work_units < PLANNING_WORK_BUDGET);
    }

    #[test]
    fn progressing_target_within_owned_defend_coverage_keeps_existing_paint() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let target_cell = IVec2::new(6, 0);
        let covered_cell = IVec2::new(5, -1);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let friendly_defenders = (0..2).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(target_cell),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
            "the moved target cell must be covered by actual owned Defend intent"
        );

        let moved_progressing = BotState {
            position: world(covered_cell),
            health: 80,
            ..target
        };
        let mut bots = bots;
        *bots.last_mut().unwrap() = moved_progressing;
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());

        let regular = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(regular.reviewed);
        assert!(regular.edits.is_empty());
        assert!(
            grid.cell(target_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
    }

    #[test]
    fn covered_movement_without_damage_gets_one_grace_before_reassessment() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let target_cell = IVec2::new(6, 0);
        let covered_cell = IVec2::new(5, -1);
        let alternative_cell = IVec2::new(8, 4);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let friendly_defenders = (0..2).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(target_cell),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
        let alternative = StructureState {
            id: 30,
            owner: enemy,
            position: world(alternative_cell),
            kind: StructureKind::Charger,
            health: 1,
            minerals: 1_000,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );

        *bots.last_mut().unwrap() = BotState {
            position: world(covered_cell),
            ..target
        };
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());
        let grace = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(grace.reviewed);
        assert!(grace.edits.is_empty());
        let reassessed = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 2,
            finished: false,
        });
        assert!(reassessed.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == alternative_cell
        }));
    }

    #[test]
    fn progressing_target_that_exits_owned_defend_coverage_is_followed() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let target_cell = IVec2::new(6, 0);
        let covered_cell = IVec2::new(5, -1);
        let uncovered_cell = IVec2::new(10, 0);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let friendly_defenders = (0..2).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(target_cell),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
        assert!(
            !grid
                .cell(uncovered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
            "the exit must begin outside actual owned Defend coverage"
        );

        *bots.last_mut().unwrap() = BotState {
            position: world(covered_cell),
            health: 80,
            ..target
        };
        let covered = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!covered.reviewed);
        assert!(covered.edits.is_empty());

        *bots.last_mut().unwrap() = BotState {
            position: world(uncovered_cell),
            health: 60,
            ..target
        };
        let exited = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 2,
            finished: false,
        });
        assert!(exited.reviewed);
        assert!(exited.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == uncovered_cell
        }));
    }

    #[test]
    fn stalled_covered_target_yields_to_a_clearly_better_plan_after_one_grace_review() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let target_cell = IVec2::new(6, 0);
        let covered_cell = IVec2::new(5, -1);
        let alternative_cell = IVec2::new(8, 4);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let friendly_defenders = (0..2).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(target_cell),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
        let alternative = StructureState {
            id: 30,
            owner: enemy,
            position: world(alternative_cell),
            kind: StructureKind::Charger,
            health: 1,
            minerals: 1_000,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );

        *bots.last_mut().unwrap() = BotState {
            position: world(covered_cell),
            health: 80,
            ..target
        };
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());
        let progress_review = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(progress_review.reviewed);
        assert!(progress_review.edits.is_empty());

        let grace = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 2,
            finished: false,
        });
        assert!(grace.reviewed);
        assert!(grace.edits.is_empty());
        let reassessed = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 3,
            finished: false,
        });
        assert!(reassessed.reviewed);
        assert!(reassessed.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == alternative_cell
        }));
    }

    #[test]
    fn renewed_covered_progress_resets_the_stall_grace() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let target_cell = IVec2::new(6, 0);
        let covered_cell = IVec2::new(5, -1);
        let alternative_cell = IVec2::new(8, 4);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let friendly_defenders = (0..2).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        });
        let target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(target_cell),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let mut bots = friendly_defenders.chain([target]).collect::<Vec<_>>();
        let alternative = StructureState {
            id: 30,
            owner: enemy,
            position: world(alternative_cell),
            kind: StructureKind::Charger,
            health: 1,
            minerals: 1_000,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );

        *bots.last_mut().unwrap() = BotState {
            position: world(covered_cell),
            health: 80,
            ..target
        };
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());
        let progress_review = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(progress_review.edits.is_empty());
        let first_grace = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 2,
            finished: false,
        });
        assert!(first_grace.edits.is_empty());

        bots.last_mut().unwrap().health = 60;
        let renewed_progress = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 3,
            finished: false,
        });
        assert!(renewed_progress.reviewed);
        assert!(renewed_progress.edits.is_empty());
        let reset_grace = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 4,
            finished: false,
        });
        assert!(reset_grace.reviewed);
        assert!(reset_grace.edits.is_empty());
        let reassessed = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[alternative],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 5,
            finished: false,
        });
        assert!(reassessed.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == alternative_cell
        }));
    }

    #[test]
    fn covered_progress_restores_owned_defend_lost_by_another_target() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let bots = (0..10)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let friendly_charger = StructureState {
            id: 20,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 100,
        };
        let moving_target = StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(6, 4)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 1_000,
        };
        let other_target = StructureState {
            id: 31,
            owner: enemy,
            position: world(IVec2::new(12, 0)),
            kind: StructureKind::Sink,
            health: 100,
            minerals: 0,
        };
        let other_cell = world_to_intent_cell(other_target.position);
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, moving_target, other_target],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(other_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
            "the fixture must establish the second attack obligation"
        );
        let moving_cell = world_to_intent_cell(moving_target.position);
        let covered_cell = grid
            .iter_active_cells()
            .map(|(cell, _)| cell)
            .find(|cell| {
                *cell != moving_cell
                    && (*cell - moving_cell).abs().max_element() <= 2
                    && grid
                        .cell(*cell)
                        .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
            })
            .expect("the moving target needs another actually covered cell");
        let moving_target = StructureState {
            position: world(covered_cell),
            health: 80,
            ..moving_target
        };
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, moving_target, other_target],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());

        grid.erase(other_cell, IntentKind::Defend, owner);
        let replanned = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, moving_target, other_target],
            deposits: &[],
            terrain: &[],
            tick: 2,
            finished: false,
        });
        assert!(replanned.reviewed);
        assert!(replanned.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == other_cell
        }));
    }

    #[test]
    fn covered_progress_reduces_an_attack_front_when_ready_force_falls() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let bots = (0..10)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        let friendly_charger = StructureState {
            id: 20,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 100,
        };
        let moving_target = StructureState {
            id: 30,
            owner: enemy,
            position: world(IVec2::new(6, 4)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 1_000,
        };
        let other_target = StructureState {
            id: 31,
            owner: enemy,
            position: world(IVec2::new(12, 0)),
            kind: StructureKind::Sink,
            health: 100,
            minerals: 0,
        };
        let other_cell = world_to_intent_cell(other_target.position);
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, moving_target, other_target],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        let moving_cell = world_to_intent_cell(moving_target.position);
        let covered_cell = grid
            .iter_active_cells()
            .map(|(cell, _)| cell)
            .find(|cell| {
                *cell != moving_cell
                    && (*cell - moving_cell).abs().max_element() <= 2
                    && grid
                        .cell(*cell)
                        .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
            })
            .expect("the moving target needs another actually covered cell");
        let moving_target = StructureState {
            position: world(covered_cell),
            health: 80,
            ..moving_target
        };
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[friendly_charger, moving_target, other_target],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());

        let depleted_defenders = bots
            .iter()
            .map(|bot| BotState {
                charge: 0.1,
                ..*bot
            })
            .collect::<Vec<_>>();
        let empty_charger = StructureState {
            minerals: 0,
            ..friendly_charger
        };
        let regular = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &depleted_defenders,
            structures: &[empty_charger, moving_target, other_target],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(regular.reviewed);
        assert!(!regular.edits.is_empty());
        apply(&mut grid, owner, &regular);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
        assert!(
            !grid
                .cell(other_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
    }

    #[test]
    fn covered_progress_opens_a_new_front_when_ready_force_grows() {
        let mut grid = IntentGrid::new(32, 32);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let target_cell = IVec2::new(6, 0);
        let covered_cell = IVec2::new(5, -1);
        let new_target_cell = IVec2::new(10, 4);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 2_000,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(12, 0)),
                minerals: 0,
            },
        ];
        let target = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(target_cell),
            health: 100,
            charge: 1.0,
            cargo: 0,
        };
        let mut bots = (0..2)
            .map(|id| BotState {
                id,
                owner,
                kind: NanobotType::Defender,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .chain([target])
            .collect::<Vec<_>>();
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );

        *bots.last_mut().unwrap() = BotState {
            position: world(covered_cell),
            health: 80,
            ..target
        };
        let immediate = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[],
            deposits: &[],
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(!immediate.reviewed);
        assert!(immediate.edits.is_empty());

        bots.truncate(2);
        bots.extend((2..8).map(|id| BotState {
            id,
            owner,
            kind: NanobotType::Defender,
            position: swarms[0].home,
            health: 100,
            charge: 1.0,
            cargo: 0,
        }));
        bots.push(BotState {
            position: world(covered_cell),
            health: 60,
            ..target
        });
        let supplied_charger = StructureState {
            id: 20,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 100,
        };
        let no_fresh_target = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[supplied_charger],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(no_fresh_target.reviewed);
        assert!(no_fresh_target.edits.is_empty());

        bots.last_mut().unwrap().health = 40;
        let new_target = StructureState {
            id: 30,
            owner: enemy,
            position: world(new_target_cell),
            kind: StructureKind::Charger,
            health: 20,
            minerals: 1_000,
        };
        let expanded = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &bots,
            structures: &[supplied_charger, new_target],
            deposits: &[],
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS * 2,
            finished: false,
        });
        apply(&mut grid, owner, &expanded);
        assert!(
            grid.cell(covered_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
        assert!(
            grid.cell(new_target_cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner))
        );
    }

    #[test]
    fn structure_free_remnant_is_followed_across_cells_and_after_disappearance() {
        let mut grid = IntentGrid::new(64, 64);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 100,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(20, 20)),
                minerals: 0,
            },
        ];
        let deposit = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let remnant = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(IVec2::new(12, 3)),
            health: 20,
            charge: 1.0,
            cargo: 0,
        };
        let mut controller = Controller::adaptive(owner);
        let first = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        assert!(first.explanation.contains("hunt remaining Nanobot 50"));
        assert!(first.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(12, 3)
        }));
        apply(&mut grid, owner, &first);

        let moved = BotState {
            position: world(IVec2::new(13, 3)),
            ..remnant
        };
        let relocated = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[moved],
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            tick: 1,
            finished: false,
        });
        assert!(relocated.reviewed);
        assert!(relocated.explanation.contains("moved"));
        assert!(relocated.edits.iter().any(|edit| {
            edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(13, 3)
        }));
        apply(&mut grid, owner, &relocated);

        let disappeared = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[],
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            tick: 2,
            finished: false,
        });
        assert!(disappeared.reviewed);
        assert!(disappeared.explanation.contains("disappeared"));
        assert!(disappeared.edits.iter().any(|edit| {
            !edit.paint && edit.kind == IntentKind::Defend && edit.cell == IVec2::new(13, 3)
        }));
    }

    #[test]
    fn controllers_stop_planning_after_the_match_finishes() {
        let grid = IntentGrid::new(16, 16);
        let state = GameState {
            finished: true,
            ..empty_state(&grid, 0)
        };

        for mut controller in [
            Controller::adaptive(SwarmId(4)),
            Controller::timed(SwarmId(4), IVec2::new(3, 3), IVec2::ZERO, 0, 1),
        ] {
            let decision = controller.decide(&state);
            assert!(!decision.reviewed);
            assert!(decision.edits.is_empty());
            assert_eq!(decision.work_units, 0);
        }
    }

    #[test]
    fn regular_and_urgent_reviews_share_one_work_allowance_until_the_next_window() {
        let mut grid = IntentGrid::new(64, 64);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let swarms = [
            SwarmState {
                id: owner,
                home: world(IVec2::ZERO),
                minerals: 100,
            },
            SwarmState {
                id: enemy,
                home: world(IVec2::new(24, 24)),
                minerals: 0,
            },
        ];
        let deposit = [DepositState {
            id: 20,
            position: world(IVec2::new(-1, 0)),
            amount: 500,
            radius: 100.0,
        }];
        let remnant = BotState {
            id: 50,
            owner: enemy,
            kind: NanobotType::Worker,
            position: world(IVec2::new(12, 3)),
            health: 20,
            charge: 1.0,
            cargo: 0,
        };
        let charger = StructureState {
            id: 42,
            owner,
            position: world(IVec2::new(1, 0)),
            kind: StructureKind::Charger,
            health: 100,
            minerals: 25,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &[charger],
            deposits: &deposit,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);

        let no_charger = (0..60_000)
            .map(|id| StructureState {
                id: 1_000 + id,
                owner,
                position: swarms[0].home,
                kind: StructureKind::Planned,
                health: 100,
                minerals: 0,
            })
            .collect::<Vec<_>>();
        let charger_loss = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &[remnant],
            structures: &no_charger,
            deposits: &deposit,
            terrain: &[],
            tick: 1,
            finished: false,
        });
        apply(&mut grid, owner, &charger_loss);
        assert!(charger_loss.reviewed);

        let moved = BotState {
            position: world(IVec2::new(13, 3)),
            ..remnant
        };
        let mut crowded = (0..60_000)
            .map(|id| BotState {
                id: 1_000 + id,
                owner,
                kind: NanobotType::Worker,
                position: swarms[0].home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            })
            .collect::<Vec<_>>();
        crowded.push(moved);
        let target_move = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &crowded,
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            tick: 2,
            finished: false,
        });

        assert!(
            initial.work_units + charger_loss.work_units + target_move.work_units
                <= PLANNING_WORK_BUDGET,
            "reviews in one window spent {}, {}, and {} work units",
            initial.work_units,
            charger_loss.work_units,
            target_move.work_units
        );
        assert!(
            !target_move.reviewed,
            "exhausted work must defer replanning"
        );
        assert!(
            target_move.edits.is_empty(),
            "deferral must preserve owned paint"
        );

        let recovered = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &crowded,
            structures: &[],
            deposits: &deposit,
            terrain: &[],
            tick: REVIEW_PERIOD_TICKS,
            finished: false,
        });
        assert!(
            recovered.reviewed,
            "a fresh window must restore planning work"
        );
        assert!(recovered.work_units > target_move.work_units);
    }

    #[test]
    fn mature_funded_force_retains_attack_and_primary_mining_for_a_minute() {
        let mut grid = IntentGrid::new(96, 96);
        let owner = SwarmId(4);
        let enemy = SwarmId(9);
        let home = world(IVec2::ZERO);
        let enemy_home = world(IVec2::new(32, 0));
        let swarms = [
            SwarmState {
                id: owner,
                home,
                minerals: 0,
            },
            SwarmState {
                id: enemy,
                home: enemy_home,
                minerals: 0,
            },
        ];
        let economy_bots = mature_economy_bots(owner, home);
        let deposits = [
            DepositState {
                id: 20,
                position: world(IVec2::new(-1, 0)),
                amount: 72_000,
                radius: 64.0,
            },
            DepositState {
                id: 21,
                position: world(IVec2::new(4, 0)),
                amount: 72_000,
                radius: 64.0,
            },
        ];
        let primary_chain = funded_primary_chain(owner, home, deposits[0]);
        let enemy_facility = StructureState {
            id: 40,
            owner: enemy,
            position: enemy_home,
            kind: StructureKind::Facility,
            health: 100,
            minerals: 0,
        };
        let mut controller = Controller::adaptive(owner);
        let initial = controller.decide(&GameState {
            grid: &grid,
            swarms: &swarms,
            bots: &economy_bots,
            structures: &[primary_chain[0], primary_chain[1], enemy_facility],
            deposits: &deposits,
            terrain: &[],
            tick: 0,
            finished: false,
        });
        apply(&mut grid, owner, &initial);
        assert!(
            grid.cell(IVec2::new(-1, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
        );
        assert!(
            !grid
                .cell(IVec2::new(4, 0))
                .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner))
        );

        let supplied_swarms = [
            SwarmState {
                minerals: 2_000,
                ..swarms[0]
            },
            swarms[1],
        ];
        let bots = economy_bots
            .into_iter()
            .chain((0..10).map(|id| BotState {
                id: 100 + id,
                owner,
                kind: NanobotType::Defender,
                position: home,
                health: 100,
                charge: 1.0,
                cargo: 0,
            }))
            .collect::<Vec<_>>();
        for review in 1..=120 {
            let pressure = controller.decide(&GameState {
                grid: &grid,
                swarms: &supplied_swarms,
                bots: &bots,
                structures: &[primary_chain[0], primary_chain[1], enemy_facility],
                deposits: &deposits,
                terrain: &[],
                tick: review * REVIEW_PERIOD_TICKS,
                finished: false,
            });
            assert!(pressure.reviewed);
            assert!(
                pressure.explanation.contains("target [32, 0]"),
                "pressure decision lacked diagnostic coordinates: {}",
                pressure.explanation
            );
            apply(&mut grid, owner, &pressure);
            assert!(grid.iter_active_cells().any(|(cell, intent)| {
                cell != IVec2::ZERO
                    && cell.abs().max_element() <= 1
                    && intent.has_owned(IntentKind::Build, owner)
            }));
            assert!(
                grid.cell(IVec2::new(32, 0))
                    .is_some_and(|intent| intent.has_owned(IntentKind::Defend, owner)),
                "pressure disappeared at review {review}"
            );
            assert!(
                grid.cell(IVec2::new(-1, 0))
                    .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner)),
                "pressure abandoned the working primary deposit"
            );
            assert!(
                !grid
                    .cell(IVec2::new(4, 0))
                    .is_some_and(|intent| intent.has_owned(IntentKind::Gather, owner)),
                "pressure opened a second resource site"
            );
        }
    }
}
