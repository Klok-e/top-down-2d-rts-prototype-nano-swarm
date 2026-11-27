use super::{
    economy::{EconomyPlan, choose_claim, observe_contested, plan_economy},
    materialize::{intent_edits, materialize_plan, world_to_intent_cell},
    raid::RaidForces,
    staging::{StageFailure, review_staging},
    strategy::{generate_candidates, validate_plan},
    *,
};

#[derive(Debug, Default)]
pub(super) struct PlannerState {
    tuning_review_requested: bool,
    last_review_tick: Option<u64>,
    work_window_start_tick: Option<u64>,
    work_used: usize,
    cleanup_cursor: Option<IVec2>,
    unreachable_cleanup_cursor: Option<IVec2>,
    last_grid_revision: Option<u64>,
    retained_covered_movement: bool,
    covered_stall_grace_used: bool,
    current: Option<IntentPlan>,
    denied_deposits: BTreeSet<(u64, SwarmId)>,
}

#[derive(Debug)]
pub(super) struct WorkBudget {
    pub(super) used: usize,
    call_start: usize,
    limit: usize,
    pub(super) exhausted: bool,
}

impl WorkBudget {
    pub(super) fn planning(already_used: usize) -> Self {
        Self {
            used: already_used,
            call_start: already_used,
            limit: PLANNING_WORK_BUDGET - EDIT_WORK_RESERVE,
            exhausted: false,
        }
    }

    fn release_edit_reserve(&mut self, plan_work: usize) {
        const { assert!(2 * MAX_PLAN_CELLS <= EDIT_WORK_RESERVE) };
        self.limit = PLANNING_WORK_BUDGET;
        assert!(
            self.spend(plan_work),
            "plan edits must fit in the edit reserve"
        );
    }

    pub(super) fn spend(&mut self, amount: usize) -> bool {
        if self.used.saturating_add(amount) > self.limit {
            self.exhausted = true;
            false
        } else {
            self.used += amount;
            true
        }
    }

    pub(super) fn optional_probe(
        &mut self,
        max_work: usize,
        probe: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        let limit = self.used.saturating_add(max_work);
        // Leave planning room for required paint after a full optional probe.
        if self.exhausted || self.limit.saturating_sub(limit) < max_work {
            return false;
        }
        let mut probe_budget = Self { limit, ..*self };
        let result = probe(&mut probe_budget);
        self.used = probe_budget.used;
        result
    }

    fn call_work(&self) -> usize {
        self.used.saturating_sub(self.call_start)
    }
}

impl Controller {
    pub fn new(owner: SwarmId) -> Self {
        Self {
            owner,
            state: PlannerState::default(),
        }
    }

    pub fn request_tuning_review(&mut self) {
        self.state.tuning_review_requested = true;
    }

    pub fn owner(&self) -> SwarmId {
        self.owner
    }

    pub(crate) fn retained_mine_cells(&self) -> impl Iterator<Item = (u64, [IVec2; 2])> + '_ {
        self.state.current.iter().flat_map(|plan| {
            plan.owned_deposits
                .iter()
                .map(|mine| (mine.id, [mine.gather_cell, mine.build_cell]))
        })
    }

    pub fn decide(&mut self, tuning: &AutonomyTuning, state: &GameState<'_>) -> Decision {
        if state.finished {
            return Decision::idle("match finished");
        }
        self.state.decide(self.owner, tuning, state)
    }
}

impl PlannerState {
    fn describe_active_plan(&self, reason: &str) -> String {
        let Some(plan) = &self.current else {
            return reason.to_string();
        };
        let mut explanation = format!(
            "{reason}, the active objective is to {}",
            plan.objective.label()
        );
        plan.append_context(&mut explanation);
        explanation
    }

    fn decide(
        &mut self,
        owner: SwarmId,
        tuning: &AutonomyTuning,
        state: &GameState<'_>,
    ) -> Decision {
        if self
            .last_grid_revision
            .is_some_and(|revision| state.grid.revision() < revision)
        {
            self.cleanup_cursor = None;
            self.unreachable_cleanup_cursor = None;
        }
        self.last_grid_revision = Some(state.grid.revision());
        let window_start = state.tick - state.tick % WORK_WINDOW_TICKS;
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
            lost_deposit,
            invalidation,
            attack_progressed,
            covered_attack,
            covered_attack_movement,
        } = validation;
        let invalidated = invalidation.is_some() || lost_deposit.is_some();
        let tuning_review = std::mem::take(&mut self.tuning_review_requested);
        let regular_review = tuning_review
            || self.last_review_tick.is_none_or(|last| {
                state.tick < last
                    || state.tick.saturating_sub(last)
                        >= tuning.controller_review_period_ticks.max(1)
            });
        let tracked_movement = self.retained_covered_movement || covered_attack_movement;
        let stall_grace = !tuning_review
            && invalidation.is_none()
            && covered_attack
            && tracked_movement
            && !attack_progressed
            && !self.covered_stall_grace_used;
        if !regular_review && invalidation.is_none() && lost_deposit.is_none() {
            self.retained_covered_movement = tracked_movement;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: "current Intent Plan remains between reviews".to_string(),
                reviewed: false,
                work_units: budget.call_work(),
            };
        }

        let Some((mut economy, mut expansion)) =
            plan_economy(owner, state, self.current.as_ref(), &mut budget)
        else {
            self.tuning_review_requested |= tuning_review;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: self.describe_active_plan(if budget.exhausted {
                    "planning allowance exhausted while reviewing owned Resource Deposits"
                } else {
                    "owned Resource Deposit support would exceed available paint"
                }),
                reviewed: true,
                work_units: budget.call_work(),
            };
        };
        let current = self.current.as_ref();
        let home_candidate = PlanCandidate::hold_home(world_to_intent_cell(expansion.home));
        let mut incumbent_stage = None;
        let mut denied_deposits = None;
        let materialize_home = |economy: &EconomyPlan, budget: &mut WorkBudget| {
            materialize_plan(&home_candidate, economy, None, owner, state, tuning, budget)
        };
        let (plan, mut explanation) = 'review: {
            let mut fallback = if invalidated || current.is_none() {
                materialize_home(&economy, &mut budget)
            } else {
                None
            };
            let defending_mine = economy.threatened.is_some();
            if budget.exhausted
                || !defending_mine
                    && observe_contested(&mut expansion, &mut economy, owner, state, &mut budget)
                        .is_none()
            {
                break 'review with_fallback(
                    fallback,
                    "planning allowance exhausted while observing neutral Resource Deposits",
                );
            }
            let mut reviewed_denials = self
                .denied_deposits
                .iter()
                .take_while(|_| budget.spend(1))
                .filter(|claim| economy.enemy_gather_claims.contains(claim))
                .copied()
                .collect();
            let mut candidates = generate_candidates(
                tuning,
                owner,
                state,
                &economy,
                &mut reviewed_denials,
                &mut budget,
            );
            let candidate_count = candidates.len();
            if budget.exhausted
                || !budget
                    .spend(candidate_count.saturating_mul(candidate_count.ilog2() as usize + 1))
            {
                break 'review with_fallback(
                    fallback,
                    "planning allowance exhausted while comparing objectives",
                );
            }
            candidates.sort_by(|left, right| {
                left.objective
                    .priority()
                    .cmp(&right.objective.priority())
                    .then_with(|| left.score.total_cmp(&right.score))
            });
            let raid_forces = if candidates
                .iter()
                .any(|candidate| candidate.objective.needs_ready_force())
            {
                RaidForces::observe(owner, state, current, &mut budget)
            } else {
                RaidForces::default()
            };
            let current_raid = current
                .map(|plan| plan.objective)
                .filter(|objective| objective.needs_ready_force());
            if budget.exhausted {
                break 'review with_fallback(
                    fallback,
                    "planning allowance exhausted while observing raid forces",
                );
            }

            // Confirm the incumbent's readiness before routing alternatives.
            let incumbent_index = current_raid.and_then(|objective| {
                candidates
                    .iter()
                    .enumerate()
                    .rev()
                    .take_while(|_| budget.spend(1))
                    .find_map(|(index, candidate)| {
                        (candidate.objective == objective).then_some(index)
                    })
            });
            let mut incumbent_ineligible = false;
            if let Some(index) = incumbent_index {
                match review_staging(
                    tuning,
                    &candidates[index],
                    current,
                    owner,
                    state,
                    &raid_forces,
                    &mut budget,
                ) {
                    Ok(front) => incumbent_stage = Some(front),
                    Err(StageFailure::WorkExhausted) => (),
                    Err(StageFailure::Ineligible | StageFailure::SustainedReadinessLoss) => {
                        incumbent_ineligible = true;
                        if fallback.is_none() {
                            fallback = materialize_home(&economy, &mut budget);
                        }
                    }
                }
            }

            let mut secured_mine = None;
            let mut stage_candidate = |index, candidate: &PlanCandidate| {
                if candidate.attack_target.is_none() {
                    return Some(None);
                }
                if budget.exhausted
                    || matches!(candidate.objective, Objective::DenyDeposit(id) if secured_mine == Some(id))
                {
                    return None;
                }
                let front = if incumbent_index == Some(index) {
                    incumbent_stage.clone()?
                } else {
                    review_staging(
                        tuning,
                        candidate,
                        current,
                        owner,
                        state,
                        &raid_forces,
                        &mut budget,
                    )
                    .ok()?
                };
                if let Objective::DenyDeposit(id) = candidate.objective
                    && !front.waiting_for_readiness()
                    && current
                        .filter(|plan| plan.objective == candidate.objective)
                        .and_then(|plan| plan.staging.as_ref())
                        .is_some_and(|previous| {
                            previous.point() == front.point()
                                && previous.destination_secured(
                                    owner,
                                    state,
                                    tuning.strategy_staging_arrival_fraction,
                                    &mut budget,
                                )
                        })
                {
                    secured_mine = Some(id);
                    return None;
                }
                Some(Some(front))
            };

            let (best_index, best_staging) = candidates
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, candidate)| {
                    stage_candidate(index, candidate).map(|staging| (index, staging))
                })
                .expect("the home economy is eligible");
            let best = &candidates[best_index];

            let progressing_attack = attack_progressed && !tuning_review;
            let retained = current
                .filter(|plan| plan.objective.priority() == best.objective.priority())
                .and_then(|plan| {
                    candidates[..=best_index].iter().enumerate().rev().find_map(
                        |(index, candidate)| {
                            if candidate.objective != plan.objective {
                                return None;
                            }
                            let normal_retention = progressing_attack
                                || invalidation.is_none()
                                    && (stall_grace
                                        || best.score
                                            <= candidate.score
                                                + candidate.score.abs()
                                                    * (tuning.controller_switch_margin - 1.0)
                                                + tuning.controller_switch_bias);
                            if !normal_retention && !candidate.objective.needs_ready_force() {
                                return None;
                            }
                            let staging = if index == best_index {
                                best_staging.clone()
                            } else {
                                stage_candidate(index, candidate)?
                            };
                            let waiting = staging
                                .as_ref()
                                .is_some_and(|front| front.waiting_for_readiness());
                            (normal_retention || waiting).then_some((candidate, staging))
                        },
                    )
                });
            if let Some(id) = secured_mine {
                incumbent_stage = None;
                for (deposit, enemy) in &economy.enemy_deposits {
                    if !budget.spend(1) {
                        break;
                    }
                    if deposit.id == id {
                        reviewed_denials.insert((id, *enemy));
                        break;
                    }
                }
            }
            let (selected, staging, explanation) = if let Some((retained, staging)) = retained {
                let label = retained.objective.label();
                let explanation = if staging
                    .as_ref()
                    .is_some_and(|front| front.waiting_for_readiness())
                {
                    format!("held {label} while the raid force recovers")
                } else if progressing_attack {
                    format!("retained {label}; attack is progressing")
                } else if stall_grace {
                    format!("retained {label}; covered attack is in its stall grace review")
                } else {
                    format!("retained viable {label}; no alternative was clearly better")
                };
                (retained, staging, explanation)
            } else {
                let label = best.objective.label();
                let explanation = if invalidation.is_some() {
                    format!("selected {label} after urgent review")
                } else if current.is_some() {
                    format!("selected {label} because it is clearly better than the current plan")
                } else {
                    format!("selected {label} from economic, support, and combat alternatives")
                };
                (best, best_staging, explanation)
            };

            if budget.exhausted {
                break 'review with_fallback(
                    fallback,
                    "planning allowance exhausted while choosing staging points",
                );
            }
            denied_deposits = Some(reviewed_denials);

            let replace_invalid_plan = invalidated
                || incumbent_ineligible
                || current_raid.is_some_and(|objective| {
                    objective.priority() >= selected.objective.priority()
                        && incumbent_stage.is_none()
                });
            let urgent_defense = matches!(selected.objective, Objective::DefendDeposit(_));
            // Finish a plan without the optional claim before spending the
            // remaining allowance on the combined plan.
            if (urgent_defense || (current.is_some() && replace_invalid_plan))
                && fallback
                    .as_ref()
                    .is_none_or(|plan| plan.objective != selected.objective)
                && let Some(plan) = materialize_plan(
                    selected,
                    &economy,
                    staging.as_ref(),
                    owner,
                    state,
                    tuning,
                    &mut budget,
                )
            {
                fallback = Some(plan);
            }
            if budget.exhausted {
                break 'review with_fallback(
                    fallback,
                    "planning allowance exhausted during paint fulfillment",
                );
            }
            if defending_mine
                && observe_contested(&mut expansion, &mut economy, owner, state, &mut budget)
                    .is_none()
                || choose_claim(expansion, &mut economy, tuning, owner, state, &mut budget)
                    .is_none()
            {
                break 'review with_fallback(
                    fallback,
                    "planning allowance exhausted while reviewing a new Resource Deposit claim",
                );
            }

            let plan = if economy.expansion.is_none()
                && fallback
                    .as_ref()
                    .is_some_and(|plan| plan.objective == selected.objective)
            {
                fallback.take()
            } else {
                materialize_plan(
                    selected,
                    &economy,
                    staging.as_ref(),
                    owner,
                    state,
                    tuning,
                    &mut budget,
                )
            };
            if plan.is_none() {
                economy.expansion = None;
                let reason = if budget.exhausted {
                    "planning allowance exhausted during paint fulfillment"
                } else {
                    "coordinated paint could not be fulfilled within the review limits"
                };
                break 'review with_fallback(fallback, reason);
            }
            (plan, explanation)
        };
        let Some(selected) = plan else {
            if let Some(held) = self.current.as_mut().and_then(|plan| plan.staging.as_mut())
                && let Some(reviewed) = incumbent_stage.as_ref()
            {
                held.observe_readiness_from(reviewed);
            }
            self.tuning_review_requested |= tuning_review && budget.exhausted;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: self.describe_active_plan(&explanation),
                reviewed: true,
                work_units: budget.call_work(),
            };
        };
        if let Some(reason) = invalidation {
            explanation = format!("{reason}; {explanation}");
        }
        if let Some(id) = lost_deposit {
            explanation.push_str(&format!(
                ", owned Resource Deposit {id} was exhausted, lost, or unreachable"
            ));
        }
        let keeps_objective =
            current.is_some_and(|current| current.objective == selected.objective);
        let previous = self.current.as_ref().map(|plan| &plan.desired);
        budget.release_edit_reserve(previous.map_or(0, HashSet::len) + selected.desired.len());
        let edits = intent_edits(
            owner,
            state,
            previous,
            &selected.desired,
            &mut self.cleanup_cursor,
            &mut self.unreachable_cleanup_cursor,
            &mut budget,
        );
        self.work_used = budget.used;
        self.last_review_tick = Some(state.tick);
        self.retained_covered_movement = keeps_objective && covered_attack && tracked_movement;
        self.covered_stall_grace_used = self.retained_covered_movement
            && !attack_progressed
            && (stall_grace || self.covered_stall_grace_used);
        if let Some(denials) = denied_deposits {
            self.denied_deposits = denials;
        }
        if let Some(claim) = economy.expansion {
            if let Some(replacement) = claim.replacement {
                explanation.push_str(&format!(
                    ", replaced stalled Resource Deposit {} with {}",
                    replacement.retires, claim.mine.id
                ));
            } else {
                explanation.push_str(&format!(
                    ", claimed neutral Resource Deposit {}",
                    claim.mine.id
                ));
            }
        }
        selected.append_context(&mut explanation);
        self.current = Some(selected);
        Decision {
            edits,
            explanation,
            reviewed: true,
            work_units: budget.call_work(),
        }
    }
}

fn with_fallback(fallback: Option<IntentPlan>, reason: &str) -> (Option<IntentPlan>, String) {
    let explanation = fallback.as_ref().map_or_else(
        || reason.to_string(),
        |plan| {
            format!(
                "{reason}; selected {} without a new claim",
                plan.objective.label()
            )
        },
    );
    (fallback, explanation)
}
