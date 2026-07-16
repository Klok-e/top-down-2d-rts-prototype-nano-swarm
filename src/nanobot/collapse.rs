//! Production-collapse detection.
//!
//! Collapse means unmet workload exists and the swarm has neither operational
//! production nor a physically viable recovery path. Recovery facts include an
//! owned plan or Build space, Worker/Hauler capability, infrastructure condition,
//! and a material source. Crew counts alone never imply recoverability.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::nanobot::OpponentSwarm;
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::Swarm;
use crate::nanobot::production::{
    OwnerSwarm, PRODUCTION_COST_PER_BOT, ProductionFacility, ProductionPriority, SwarmProduction,
    count_swarm_nanobots_by_type, total_deficit,
};
use crate::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActionableProjection, Charger, OpportunityCategory, PlannedKind, PlannedStructure,
        PopulationDemand, SupportCondition, find_build_zone_placement,
        scaled_building_footprint_radius, world_to_cell,
    },
    resources::ResourceDeposit,
};

/// Why a swarm is or is not in Production Collapse. Stored on
/// the [`CollapseOutcome`] so callers (UI, tests, future
/// game-over screen) can distinguish "we won" from "we lost"
/// from "everything is fine" without re-deriving the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollapseReason {
    /// Default: the swarm is functioning or can recover. No
    /// collapse has been detected.
    #[default]
    NotCollapsed,
    /// Unmet demand exists, but no operational production or complete recovery
    /// path remains.
    NoRecoveryPath,
    /// No facility owned by the swarm is currently busy. The
    /// swarm still has enough nanobots to recover, so this is
    /// a warning state rather than a collapse.
    NoWorkingProduction,
    /// The swarm has at least one busy facility, so
    /// production is currently working. The reason field is
    /// kept so a caller can distinguish "production is
    /// running" from "no demand" without re-reading the
    /// inputs.
    Working,
}

/// Result of [`evaluate_recovery`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CollapseOutcome {
    pub collapsed: bool,
    pub reason: CollapseReason,
}

/// Explicit facts required to decide whether production can recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoveryFacts {
    pub has_unmet_demand: bool,
    pub operational_production: bool,
    pub viable_planned_facility: bool,
    pub recoverable_existing_facility: bool,
    pub funded_existing_facility: bool,
    pub has_worker: bool,
    pub has_hauler: bool,
    pub has_build_space: bool,
    pub has_material_path: bool,
}

/// Decide collapse from an actual production or rebuild path.
pub fn evaluate_recovery(facts: RecoveryFacts) -> CollapseOutcome {
    if !facts.has_unmet_demand {
        return CollapseOutcome::default();
    }
    if facts.operational_production {
        return CollapseOutcome {
            collapsed: false,
            reason: CollapseReason::Working,
        };
    }
    let can_finish_plan = facts.viable_planned_facility
        && facts.has_worker
        && facts.has_hauler
        && facts.has_material_path;
    let can_supply_existing = facts.recoverable_existing_facility
        && facts.has_worker
        && facts.has_hauler
        && facts.has_material_path;
    let can_repair_funded = facts.funded_existing_facility && facts.has_worker;
    let can_rebuild =
        facts.has_worker && facts.has_hauler && facts.has_build_space && facts.has_material_path;
    CollapseOutcome {
        collapsed: !(can_finish_plan || can_supply_existing || can_repair_funded || can_rebuild),
        reason: if can_finish_plan || can_supply_existing || can_repair_funded || can_rebuild {
            CollapseReason::NoWorkingProduction
        } else {
            CollapseReason::NoRecoveryPath
        },
    }
}

/// Bevy resource that records the latest collapse state for
/// each side. Read by the UI layer (or a future game-over
/// screen) to render a win/loss banner. The detection system
/// overwrites both fields every tick so callers always see
/// the most recent evaluation.
#[derive(Debug, Default, Resource, Clone, Copy)]
pub struct ProductionCollapseState {
    /// `true` when the player swarm is in Production Collapse.
    pub player_collapsed: bool,
    /// `true` when the opponent swarm is in Production Collapse.
    pub opponent_collapsed: bool,
}

impl ProductionCollapseState {
    /// Convenience: the player has won iff the opponent
    /// swarm has collapsed while the player swarm has not.
    /// "Both collapsed" is not a player win; the helpers
    /// stay separate so the UI can render the more nuanced
    /// state.
    pub fn player_won(&self) -> bool {
        self.opponent_collapsed && !self.player_collapsed
    }

    /// Convenience: the player has lost iff the player swarm
    /// has collapsed.
    pub fn player_lost(&self) -> bool {
        self.player_collapsed
    }
}

/// Evaluate explicit recovery facts for every swarm and update
/// [`ProductionCollapseState`] after production and opportunity projection.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn production_collapse_detection_system(
    swarms: Query<
        (
            Entity,
            Option<&crate::nanobot::components::SwarmId>,
            Option<&OpponentSwarm>,
        ),
        With<Swarm>,
    >,
    facilities: Query<(&ProductionFacility, &OwnerSwarm, Option<&SupportCondition>)>,
    planned: Query<(&PlannedStructure, Option<&OwnerSwarm>)>,
    support_structures: Query<
        &Transform,
        Or<(
            With<ProductionFacility>,
            With<crate::resources::Stockpile>,
            With<Charger>,
            With<PlannedStructure>,
        )>,
    >,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    material_stockpiles: Query<(
        &crate::resources::Stockpile,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    nanobots: Query<
        (
            &crate::nanobot::NanobotType,
            &crate::nanobot::components::SwarmMember,
        ),
        With<crate::nanobot::components::Nanobot>,
    >,
    global_priority: Res<ProductionPriority>,
    swarm_productions: Query<&SwarmProduction>,
    mut state: ResMut<ProductionCollapseState>,
    grid: Res<IntentGrid>,
    projection: Option<Res<ActionableProjection>>,
    population_demand: Option<Res<PopulationDemand>>,
) {
    state.player_collapsed = false;
    state.opponent_collapsed = false;
    for (swarm_entity, swarm_id, opponent) in &swarms {
        let swarm_id = swarm_id
            .copied()
            .unwrap_or(crate::nanobot::components::SwarmId::PLAYER);
        let counts = count_swarm_nanobots_by_type(swarm_id, &nanobots);
        let workers = *counts.get(&NanobotType::Worker).unwrap_or(&0);
        let haulers = *counts.get(&NanobotType::Hauler).unwrap_or(&0);
        let priority = swarm_productions
            .get(swarm_entity)
            .map(|production| &production.priority)
            .unwrap_or(&*global_priority);
        let has_unmet_demand = population_demand
            .as_deref()
            .map(|demand| demand.has_shortage(swarm_id, &counts))
            .unwrap_or_else(|| total_deficit(priority, &counts) > 0);

        let operational_production = facilities.iter().any(|(facility, owner, condition)| {
            owner.0 == swarm_entity
                && condition.is_none_or(|condition| condition.is_operational())
                && (facility.is_busy() || facility.input_amount >= PRODUCTION_COST_PER_BOT)
        });
        let recoverable_existing_facility = facilities.iter().any(|(_, owner, condition)| {
            owner.0 == swarm_entity && condition.is_none_or(|condition| condition.health > 0)
        });
        let funded_existing_facility = facilities.iter().any(|(facility, owner, condition)| {
            owner.0 == swarm_entity
                && condition.is_none_or(|condition| condition.health > 0)
                && (facility.is_busy() || facility.input_amount >= PRODUCTION_COST_PER_BOT)
        });
        let viable_planned_facility = planned.iter().any(|(planned, owner)| {
            planned.kind == PlannedKind::ProductionFacility
                && owner.is_some_and(|owner| owner.0 == swarm_entity)
        });
        let occupied_cells = support_structures
            .iter()
            .map(|transform| world_to_cell(transform.translation.truncate()))
            .collect::<HashSet<_>>();
        let build_cells = grid
            .iter_active_cells()
            .filter_map(|(cell, intent)| {
                (intent.has(IntentKind::Build)
                    && intent.owner(IntentKind::Build) == Some(swarm_id)
                    && !occupied_cells.contains(&cell))
                .then_some(cell)
            })
            .collect::<Vec<_>>();
        let mut obstacles = support_structures
            .iter()
            .map(|transform| {
                (
                    transform.translation.truncate(),
                    scaled_building_footprint_radius(transform),
                )
            })
            .collect::<Vec<_>>();
        obstacles.extend(
            deposits
                .iter()
                .map(|(deposit, transform)| (transform.translation.truncate(), deposit.radius)),
        );
        let has_build_space = find_build_zone_placement(&build_cells, &obstacles, 27).is_some();
        let gather_path = projection.as_deref().is_some_and(|projection| {
            projection.iter_regions().any(|(_, opportunities)| {
                opportunities.iter().any(|opportunity| {
                    opportunity.category == OpportunityCategory::Gather
                        && opportunity.owner.is_none_or(|owner| owner == swarm_id)
                        && opportunity.available_work > 0
                })
            })
        });
        let staged_material: u32 = material_stockpiles
            .iter()
            .filter(|(_, owner, condition)| {
                (owner.is_some_and(|owner| owner.0 == swarm_entity)
                    || (owner.is_none() && swarm_id == crate::nanobot::components::SwarmId::PLAYER))
                    && condition.is_none_or(|condition| condition.is_operational())
            })
            .map(|(stockpile, _, _)| stockpile.amount)
            .sum();
        let has_material_path = staged_material >= PRODUCTION_COST_PER_BOT || gather_path;

        let outcome = evaluate_recovery(RecoveryFacts {
            has_unmet_demand,
            operational_production,
            viable_planned_facility,
            recoverable_existing_facility,
            funded_existing_facility,
            has_worker: workers > 0,
            has_hauler: haulers > 0,
            has_build_space,
            has_material_path,
        });
        if outcome.collapsed {
            if opponent.is_some() {
                state.opponent_collapsed = true;
            } else {
                state.player_collapsed = true;
            }
        }
    }
}

/// Plugin that wires production-collapse detection into the fixed simulation.
/// Auto-initialises [`ProductionCollapseState`] for consumers.
pub struct CollapsePlugin;

impl Plugin for CollapsePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProductionCollapseState>().add_systems(
            FixedUpdate,
            production_collapse_detection_system
                .after(crate::nanobot::production::production_facility_work_system)
                .after(crate::nanobot::RegionalAllocationSet::Project),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure recoverability decision.

    use super::*;

    #[test]
    fn planned_facility_with_delivery_crew_and_material_is_recoverable() {
        let outcome = evaluate_recovery(RecoveryFacts {
            has_unmet_demand: true,
            viable_planned_facility: true,
            has_worker: true,
            has_hauler: true,
            has_material_path: true,
            ..Default::default()
        });
        assert!(!outcome.collapsed);
        assert_eq!(outcome.reason, CollapseReason::NoWorkingProduction);
    }

    #[test]
    fn crew_without_space_or_material_is_not_recoverable() {
        let outcome = evaluate_recovery(RecoveryFacts {
            has_unmet_demand: true,
            has_worker: true,
            has_hauler: true,
            ..Default::default()
        });
        assert!(outcome.collapsed);
        assert_eq!(outcome.reason, CollapseReason::NoRecoveryPath);
    }

    #[test]
    fn no_demand_is_not_collapse() {
        assert_eq!(
            evaluate_recovery(RecoveryFacts::default()),
            CollapseOutcome::default()
        );
    }

    #[test]
    fn production_collapse_state_default_is_neither_collapsed() {
        // The resource must default to "no collapse" so a
        // freshly started game does not flash a win/loss
        // banner before the first tick.
        let s = ProductionCollapseState::default();
        assert!(!s.player_collapsed);
        assert!(!s.opponent_collapsed);
        assert!(!s.player_won());
        assert!(!s.player_lost());
    }

    #[test]
    fn player_wins_when_opponent_collapsed_and_player_healthy() {
        let s = ProductionCollapseState {
            opponent_collapsed: true,
            ..Default::default()
        };
        assert!(s.player_won());
        assert!(!s.player_lost());
    }

    #[test]
    fn player_loses_when_player_collapsed() {
        let s = ProductionCollapseState {
            player_collapsed: true,
            ..Default::default()
        };
        assert!(s.player_lost());
        // Player_lost takes priority over player_won even
        // if both swarms happen to collapse. The UI can
        // show both flags separately for a richer state.
        let s = ProductionCollapseState {
            player_collapsed: true,
            opponent_collapsed: true,
        };
        assert!(s.player_lost());
        assert!(!s.player_won());
    }
}
