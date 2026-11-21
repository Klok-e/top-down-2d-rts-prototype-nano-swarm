use super::{
    economy::plan_economy,
    materialize::{intent_edits, materialize_plan},
    staging::plan_staging,
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

        let previous_deposits = self
            .current
            .as_ref()
            .map_or(&[][..], |plan| plan.owned_deposits.as_slice());
        let Some(economy) = plan_economy(tuning, owner, state, previous_deposits, &mut budget)
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
        let mut candidates = generate_candidates(tuning, owner, state, &economy, &mut budget);
        if budget.exhausted {
            self.tuning_review_requested |= tuning_review;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: self.describe_active_plan(
                    "planning allowance exhausted during review, retained the current Intent Plan until the next review window",
                ),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        candidates.sort_by(|left, right| {
            left.objective
                .priority()
                .cmp(&right.objective.priority())
                .then_with(|| left.score.total_cmp(&right.score))
        });
        let best = candidates
            .last()
            .expect("the home economy is a candidate")
            .clone();

        let progressing_attack = attack_progressed && best.attack.is_some() && !tuning_review;
        let refreshed = self.current.as_ref().and_then(|current| {
            candidates
                .iter()
                .find(|candidate| candidate.objective == current.objective)
        });
        let (mut selected, mut explanation) = if let Some(refreshed) = refreshed
            && refreshed.objective.priority() == best.objective.priority()
            && (progressing_attack
                || invalidation.is_none()
                    && (stall_grace
                        || best.score
                            <= refreshed.score
                                + refreshed.score.abs() * (tuning.controller_switch_margin - 1.0)
                                + tuning.controller_switch_bias))
        {
            let label = refreshed.objective.label();
            let explanation = if progressing_attack {
                format!("retained {label}; attack is progressing")
            } else if stall_grace {
                format!("retained {label}; covered attack is in its stall grace review")
            } else {
                format!("retained viable {label}; no alternative was clearly better")
            };
            (refreshed.clone(), explanation)
        } else {
            let label = best.objective.label();
            let explanation = if let Some(reason) = invalidation {
                format!("{reason}; selected {label} after urgent review")
            } else if self.current.is_some() {
                format!("selected {label} because it is clearly better than the current plan")
            } else {
                format!("selected {label} from economic, support, and combat alternatives")
            };
            (best, explanation)
        };
        let mut staging = plan_staging(
            tuning,
            &selected,
            self.current.as_ref(),
            owner,
            state,
            &mut budget,
        );
        if selected.attack_target.is_some() && staging.is_none() && !budget.exhausted {
            for alternative in candidates.iter().rev() {
                if alternative.objective == selected.objective {
                    continue;
                }
                let approach = plan_staging(
                    tuning,
                    alternative,
                    self.current.as_ref(),
                    owner,
                    state,
                    &mut budget,
                );
                if budget.exhausted {
                    break;
                }
                if alternative.attack_target.is_none() || approach.is_some() {
                    explanation = format!(
                        "selected {} because {} lacked a reachable approach",
                        alternative.objective.label(),
                        selected.objective.label(),
                    );
                    selected = alternative.clone();
                    staging = approach;
                    break;
                }
            }
        }
        if budget.exhausted {
            self.tuning_review_requested |= tuning_review;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: self.describe_active_plan(
                    "planning allowance exhausted while choosing staging points, retained the current Intent Plan until the next review window",
                ),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        if let Some(id) = lost_deposit {
            explanation.push_str(&format!(
                ", owned Resource Deposit {id} was exhausted, lost, or unreachable"
            ));
        }
        let keeps_objective = self
            .current
            .as_ref()
            .is_some_and(|current| current.objective == selected.objective);
        let selected = materialize_plan(
            selected,
            economy,
            staging,
            owner,
            state,
            tuning,
            &mut budget,
        );
        if budget.exhausted || !selected.is_coordinated() {
            self.tuning_review_requested |= tuning_review && budget.exhausted;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: self.describe_active_plan(
                    "planning allowance exhausted during paint fulfillment, so the current Intent Plan was retained",
                ),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        let previous = self
            .current
            .as_ref()
            .map_or(&[][..], |plan| plan.desired.as_slice());
        budget.release_edit_reserve(previous.len() + selected.desired.len());
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
