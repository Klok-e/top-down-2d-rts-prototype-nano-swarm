use super::{
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
        if !regular_review && invalidation.is_none() {
            self.retained_covered_movement = tracked_movement;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation: "current Intent Plan remains between reviews".to_string(),
                reviewed: false,
                work_units: budget.call_work(),
            };
        }

        let candidates =
            generate_candidates(tuning, owner, state, self.current.as_ref(), &mut budget);
        if budget.exhausted {
            self.tuning_review_requested |= tuning_review;
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

        let progressing_attack = attack_progressed && best.attack.is_some() && !tuning_review;
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
                && (progressing_attack
                    || stall_grace
                    || best.score
                        <= refreshed.score
                            + (refreshed.score - refreshed.economy.value).abs()
                                * (tuning.controller_switch_margin - 1.0)
                            + tuning.controller_switch_bias)
            {
                (
                    refreshed.clone(),
                    if progressing_attack {
                        format!(
                            "retained {}; attack is progressing",
                            refreshed.objective.label()
                        )
                    } else if stall_grace {
                        format!(
                            "retained {}; covered attack is in its stall grace review",
                            refreshed.objective.label()
                        )
                    } else {
                        format!(
                            "retained viable {}; no alternative was clearly better",
                            refreshed.objective.label()
                        )
                    },
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
        let staging = plan_staging(
            tuning,
            &selected,
            self.current.as_ref(),
            owner,
            state,
            &mut budget,
        );
        if selected.attack_target.is_some()
            && staging.is_none()
            && !budget.exhausted
            && let Some(fallback) = candidates
                .iter()
                .filter(|candidate| candidate.attack_target.is_none())
                .max_by(|left, right| left.score.total_cmp(&right.score))
        {
            selected = fallback.clone();
            explanation = format!(
                "selected {}; the military objective has no reachable approach",
                selected.objective.label()
            );
        }
        if budget.exhausted {
            self.tuning_review_requested |= tuning_review;
            self.work_used = budget.used;
            return Decision {
                edits: Vec::new(),
                explanation:
                    "planning allowance exhausted while choosing staging points; retained the current Intent Plan until the next review window"
                        .to_string(),
                reviewed: true,
                work_units: budget.call_work(),
            };
        }
        if let Some(front) = staging.as_ref() {
            explanation.push_str(&format!(
                "; staging point {} toward {}",
                front.point(),
                selected.attack_target.unwrap()
            ));
        }
        let keeps_objective = self
            .current
            .as_ref()
            .is_some_and(|current| current.objective == selected.objective);
        let selected = materialize_plan(selected, staging, owner, state, &mut budget);
        if budget.exhausted || !selected.is_coordinated() {
            self.tuning_review_requested |= tuning_review && budget.exhausted;
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
        self.current = Some(selected);
        Decision {
            edits,
            explanation,
            reviewed: true,
            work_units: budget.call_work(),
        }
    }
}
