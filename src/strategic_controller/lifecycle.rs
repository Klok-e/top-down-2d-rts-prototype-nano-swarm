use super::{
    materialize::{intent_edits, materialize_plan},
    strategy::{
        adapt_pressure_anchor, coordinate_attack_targets, generate_candidates,
        supportable_attack_fronts, validate_plan,
    },
    *,
};

#[derive(Debug, Default)]
pub(super) struct PlannerState {
    last_review_tick: Option<u64>,
    work_window_start_tick: Option<u64>,
    work_used: usize,
    cleanup_cursor: Option<IVec2>,
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

    fn release_edit_reserve(&mut self) {
        self.limit = PLANNING_WORK_BUDGET;
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

    pub fn owner(&self) -> SwarmId {
        self.owner
    }

    pub fn decide(&mut self, state: &GameState<'_>) -> Decision {
        if state.finished {
            return Decision::idle("match finished");
        }
        self.state.decide(self.owner, state)
    }
}

impl PlannerState {
    fn decide(&mut self, owner: SwarmId, state: &GameState<'_>) -> Decision {
        if self
            .last_grid_revision
            .is_some_and(|revision| state.grid.revision() < revision)
        {
            self.cleanup_cursor = None;
        }
        self.last_grid_revision = Some(state.grid.revision());
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
        let edits = intent_edits(
            owner,
            state.grid,
            &selected.desired,
            &mut self.cleanup_cursor,
            &mut budget,
        );
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
