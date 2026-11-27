use super::{
    economy::plan_economy,
    materialize::{intent_edits, materialize_plan},
    raid::RaidForces,
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

        let previous_deposits = self
            .current
            .as_ref()
            .map_or(&[][..], |plan| plan.owned_deposits.as_slice());
        let Some(mut economy) = plan_economy(tuning, owner, state, previous_deposits, &mut budget)
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
        let mut denied_deposits = self
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
            &mut denied_deposits,
            &mut budget,
        );
        let candidate_count = candidates.len();
        if budget.exhausted
            || !budget.spend(candidate_count.saturating_mul(candidate_count.ilog2() as usize + 1))
        {
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
        let raid_forces = if candidates
            .iter()
            .any(|candidate| candidate.objective.requires_raid_readiness())
        {
            RaidForces::observe(owner, state, &mut budget)
        } else {
            RaidForces::default()
        };
        let current = self.current.as_ref();
        let current_raid = current
            .map(|plan| plan.objective)
            .filter(|objective| objective.requires_raid_readiness());
        let mut incumbent_stage = None;
        let mut secured_mine = None;
        let mut stage_candidate = |candidate: &PlanCandidate| {
            if candidate.attack_target.is_none() {
                return Some(None);
            }
            if budget.exhausted
                || matches!(candidate.objective, Objective::DenyDeposit(id) if secured_mine == Some(id))
            {
                return None;
            }
            let front = plan_staging(
                tuning,
                candidate,
                current,
                owner,
                state,
                &raid_forces,
                &mut budget,
            )?;
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
            if current_raid == Some(candidate.objective) && incumbent_stage.is_none() {
                incumbent_stage = Some(front.clone());
            }
            Some(Some(front))
        };

        let (best_index, best_staging) = candidates
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, candidate)| {
                stage_candidate(candidate).map(|staging| (index, staging))
            })
            .expect("the home economy is eligible");
        let best = &candidates[best_index];

        let progressing_attack = attack_progressed && !tuning_review;
        let retained =
            current
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
                            if !normal_retention && !candidate.objective.requires_raid_readiness() {
                                return None;
                            }
                            let staging = if index == best_index {
                                best_staging.clone()
                            } else {
                                stage_candidate(candidate)?
                            };
                            let waiting = staging
                                .as_ref()
                                .is_some_and(|front| front.waiting_for_readiness());
                            (normal_retention || waiting).then_some((candidate, staging))
                        },
                    )
                });
        if let Some(id) = secured_mine {
            for (deposit, enemy) in &economy.enemy_deposits {
                if !budget.spend(1) {
                    break;
                }
                if deposit.id == id {
                    denied_deposits.insert((id, *enemy));
                    break;
                }
            }
        }
        let (selected, staging, mut explanation) = if let Some((retained, staging)) = retained {
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
            let explanation = if let Some(reason) = invalidation {
                format!("{reason}; selected {label} after urgent review")
            } else if current.is_some() {
                format!("selected {label} because it is clearly better than the current plan")
            } else {
                format!("selected {label} from economic, support, and combat alternatives")
            };
            (best, best_staging, explanation)
        };
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
        let mut plan = materialize_plan(
            selected,
            &economy,
            staging.as_ref(),
            owner,
            state,
            tuning,
            &mut budget,
        );
        let replace_invalid_plan = invalidated
            || current_raid.is_some_and(|objective| {
                objective.priority() >= selected.objective.priority() && incumbent_stage.is_none()
            });
        let urgent_defense = matches!(selected.objective, Objective::DefendDeposit(_));
        if plan.is_none()
            && (urgent_defense || (current.is_some() && replace_invalid_plan))
            && !budget.exhausted
            && economy.expansion.take().is_some()
        {
            plan = materialize_plan(
                selected,
                &economy,
                staging.as_ref(),
                owner,
                state,
                tuning,
                &mut budget,
            );
            explanation.push_str(", deferred the new claim to complete the military plan");
        }
        if plan.is_none()
            && (current.is_none() || replace_invalid_plan)
            && !budget.exhausted
            && (selected.objective != Objective::HoldHome || economy.expansion.is_some())
        {
            economy.expansion = None;
            plan = materialize_plan(
                &PlanCandidate::hold_home(selected.anchor),
                &economy,
                None,
                owner,
                state,
                tuning,
                &mut budget,
            );
            explanation =
                "selected hold the home economy because the proposed paint did not fit".to_string();
        }
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
                explanation: self.describe_active_plan(if budget.exhausted {
                    "planning allowance exhausted during paint fulfillment"
                } else {
                    "coordinated paint could not be fulfilled within the review limits"
                }),
                reviewed: true,
                work_units: budget.call_work(),
            };
        };
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
        self.denied_deposits = denied_deposits;
        if let Some(mine) = economy.expansion {
            explanation.push_str(&format!(", claimed neutral Resource Deposit {}", mine.id));
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
