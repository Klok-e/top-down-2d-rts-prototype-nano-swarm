//! Production-collapse detection.
//!
//! Collapse means unmet workload exists and the swarm has neither operational
//! production nor a physically viable recovery path. Recovery facts include an
//! owned plan or Build space, Worker/Hauler capability, infrastructure condition,
//! and a material source. Crew counts alone never imply recoverability.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::nanobot::OpponentSwarm;
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::{Swarm, SwarmId};
use crate::nanobot::production::{
    OwnerSwarm, PRODUCTION_COST_PER_BOT, ProductionFacility, ProductionPriority, SwarmProduction,
    count_swarm_nanobots_by_type, facility_belongs_to_swarm, total_deficit,
};
use crate::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActionableProjection, BUILDING_FOOTPRINT_RADIUS, Cargo, Charger, HaulerAssignment,
        LogisticsReservation, OpportunityCategory, OpportunityTarget, PlannedKind,
        PlannedStructure, PopulationDemand, SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
        SOURCE_STOCKPILE_PROXIMITY_RADIUS, SupportCondition, SwarmMember,
        find_build_zone_placement, find_source_stockpile_placement_for_demand,
        scaled_building_footprint_radius, sink_stockpile_zone_cells, world_to_cell,
    },
    resources::{ResourceDeposit, ResourceKind, StockpileRole},
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
    pub existing_facility_material_path: bool,
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
        && (facts.has_material_path || facts.existing_facility_material_path);
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

/// Stable terminal result for one skirmish. Once set, later simulation changes
/// cannot reopen the match or replace the first result.
#[derive(Debug, Default, Resource, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    #[default]
    InProgress,
    Victory,
    Defeat,
}

pub fn latch_match_outcome(
    current: MatchOutcome,
    collapse: ProductionCollapseState,
) -> MatchOutcome {
    if current != MatchOutcome::InProgress {
        current
    } else if collapse.player_lost() {
        MatchOutcome::Defeat
    } else if collapse.player_won() {
        MatchOutcome::Victory
    } else {
        MatchOutcome::InProgress
    }
}

pub fn match_outcome_latch_system(
    collapse: Res<ProductionCollapseState>,
    mut outcome: ResMut<MatchOutcome>,
) {
    *outcome = latch_match_outcome(*outcome, *collapse);
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
            Option<&Transform>,
        ),
        With<Swarm>,
    >,
    facilities: Query<(
        &ProductionFacility,
        Option<&OwnerSwarm>,
        &Transform,
        Option<&SupportCondition>,
    )>,
    planned: Query<(&PlannedStructure, &Transform, Option<&OwnerSwarm>)>,
    support_structures: Query<
        &Transform,
        Or<(
            With<ProductionFacility>,
            With<crate::resources::Stockpile>,
            With<Charger>,
            With<PlannedStructure>,
        )>,
    >,
    deposits: Query<(Entity, &ResourceDeposit, &Transform)>,
    material_stockpiles: Query<(
        Entity,
        &crate::resources::Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    facility_obstacles: Query<&Transform, With<ProductionFacility>>,
    charger_obstacles: Query<&Transform, With<Charger>>,
    cargo: Query<
        (
            &Cargo,
            &SwarmMember,
            &NanobotType,
            Option<&HaulerAssignment>,
            Option<&LogisticsReservation>,
        ),
        With<crate::nanobot::components::Nanobot>,
    >,
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
    let swarm_by_id: HashMap<SwarmId, Entity> = swarms
        .iter()
        .filter_map(|(entity, id, _, _)| id.map(|id| (*id, entity)))
        .collect();
    for (swarm_entity, swarm_id, opponent, swarm_transform) in &swarms {
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

        let operational_production = facilities.iter().any(|(facility, owner, _, condition)| {
            facility_belongs_to_swarm(owner, swarm_entity, swarm_id)
                && condition.is_none_or(|condition| condition.is_operational())
                && (facility.is_busy() || facility.input_amount >= PRODUCTION_COST_PER_BOT)
        });
        let recoverable_existing_facility = facilities.iter().any(|(_, owner, _, condition)| {
            owner.is_some_and(|owner| owner.0 == swarm_entity)
                && condition.is_none_or(|condition| condition.health > 0)
        });
        let funded_existing_facility = facilities.iter().any(|(facility, owner, _, condition)| {
            facility_belongs_to_swarm(owner, swarm_entity, swarm_id)
                && condition.is_none_or(|condition| condition.health > 0)
                && (facility.is_busy() || facility.input_amount >= PRODUCTION_COST_PER_BOT)
        });
        let viable_planned_facility = planned.iter().any(|(planned, _, owner)| {
            planned.kind == PlannedKind::ProductionFacility
                && owner.is_some_and(|owner| owner.0 == swarm_entity)
        });
        let viable_planned_sink = planned.iter().any(|(planned, _, owner)| {
            planned.kind == PlannedKind::SinkStockpile
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
                .map(|(_, deposit, transform)| (transform.translation.truncate(), deposit.radius)),
        );
        let facility_placement = find_build_zone_placement(&build_cells, &obstacles, 27);
        let has_build_space = facility_placement.is_some();
        let sink_placement_exists =
            |consumer_cell: IVec2, consumer_owner: Option<Entity>, obstacles: &[(Vec2, f32)]| {
                let zone_cells =
                    sink_stockpile_zone_cells(&grid, consumer_cell, consumer_owner, &swarm_by_id);
                find_build_zone_placement(&zone_cells, obstacles, 26).is_some()
            };
        let local_consumer_sink_path = facilities.iter().any(|(_, owner, transform, condition)| {
            owner.is_some_and(|owner| owner.0 == swarm_entity)
                && condition.is_none_or(|condition| condition.health > 0)
                && sink_placement_exists(
                    world_to_cell(transform.translation.truncate()),
                    Some(swarm_entity),
                    &obstacles,
                )
        }) || planned.iter().any(|(planned, transform, owner)| {
            planned.kind == PlannedKind::ProductionFacility
                && owner.is_some_and(|owner| owner.0 == swarm_entity)
                && sink_placement_exists(
                    world_to_cell(transform.translation.truncate()),
                    Some(swarm_entity),
                    &obstacles,
                )
        });
        let has_operational_facility = facilities.iter().any(|(_, owner, _, condition)| {
            owner.is_some_and(|owner| owner.0 == swarm_entity)
                && condition.is_none_or(|condition| condition.is_operational())
        });
        let future_facility_sink_path = !has_operational_facility
            && !viable_planned_facility
            && facility_placement.is_some_and(|(cell, position)| {
                let mut future_obstacles = obstacles.clone();
                future_obstacles.push((position, BUILDING_FOOTPRINT_RADIUS));
                sink_placement_exists(cell, Some(swarm_entity), &future_obstacles)
            });
        let stockpile_belongs_to_swarm =
            |owner: Option<&OwnerSwarm>| owner.is_some_and(|owner| owner.0 == swarm_entity);
        let mut source_obstacles = deposits
            .iter()
            .map(|(_, deposit, transform)| (transform.translation.truncate(), deposit.radius))
            .collect::<Vec<_>>();
        source_obstacles.extend(
            material_stockpiles
                .iter()
                .map(|(_, _, transform, _, _, _)| {
                    (
                        transform.translation.truncate(),
                        SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
                    )
                }),
        );
        source_obstacles.extend(planned.iter().map(|(_, transform, _)| {
            (
                transform.translation.truncate(),
                SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            )
        }));
        source_obstacles.extend(facility_obstacles.iter().map(|transform| {
            (
                transform.translation.truncate(),
                scaled_building_footprint_radius(transform),
            )
        }));
        source_obstacles.extend(charger_obstacles.iter().map(|transform| {
            (
                transform.translation.truncate(),
                scaled_building_footprint_radius(transform),
            )
        }));
        let swarm_origin = swarm_transform.map(|transform| transform.translation.truncate());
        let gather_path = projection.as_deref().is_some_and(|projection| {
            projection.iter_regions().any(|(_, opportunities)| {
                opportunities.iter().any(|opportunity| {
                    if opportunity.category != OpportunityCategory::Gather
                        || opportunity.owner.is_some_and(|owner| owner != swarm_id)
                        || opportunity.available_work == 0
                    {
                        return false;
                    }
                    let OpportunityTarget::Gather { deposit, .. } = opportunity.target else {
                        return false;
                    };
                    let Ok((_, deposit, transform)) = deposits.get(deposit) else {
                        return false;
                    };
                    if deposit.kind != ResourceKind::Minerals || deposit.amount == 0 {
                        return false;
                    }
                    let deposit_pos = transform.translation.truncate();
                    let built_source = material_stockpiles.iter().any(
                        |(_, stockpile, transform, role, owner, condition)| {
                            stockpile.kind == ResourceKind::Minerals
                                && stockpile.capacity > 0
                                && !matches!(role, Some(StockpileRole::Sink))
                                && stockpile_belongs_to_swarm(owner)
                                && condition.is_none_or(|condition| condition.is_operational())
                                && transform.translation.truncate().distance(deposit_pos)
                                    <= SOURCE_STOCKPILE_PROXIMITY_RADIUS
                        },
                    );
                    let planned_source = planned.iter().any(|(planned, transform, owner)| {
                        planned.kind == PlannedKind::SourceStockpile
                            && stockpile_belongs_to_swarm(owner)
                            && transform.translation.truncate().distance(deposit_pos)
                                <= SOURCE_STOCKPILE_PROXIMITY_RADIUS
                    });
                    built_source
                        || planned_source
                        || find_source_stockpile_placement_for_demand(
                            deposit,
                            deposit_pos,
                            swarm_id,
                            &grid,
                            &source_obstacles,
                            swarm_origin,
                        )
                        .is_some()
                })
            })
        });
        let mut source_material = 0u32;
        let mut sink_material = 0u32;
        let mut has_source_stockpile = false;
        let mut has_sink_stockpile = false;
        for (_, stockpile, _, role, owner, condition) in &material_stockpiles {
            if stockpile.kind != ResourceKind::Minerals
                || !stockpile_belongs_to_swarm(owner)
                || condition.is_some_and(|condition| !condition.is_operational())
            {
                continue;
            }
            match role.copied().unwrap_or_default() {
                StockpileRole::Source => {
                    has_source_stockpile = true;
                    source_material = source_material.saturating_add(stockpile.amount);
                }
                StockpileRole::Sink => {
                    has_sink_stockpile = true;
                    sink_material = sink_material.saturating_add(stockpile.amount);
                }
            }
        }
        for (load, member, kind, assignment, reservation) in &cargo {
            if member.0 != swarm_id || load.kind != ResourceKind::Minerals || load.amount == 0 {
                continue;
            }
            let Some(reservation) = reservation.filter(|reservation| {
                reservation.kind == load.kind && reservation.amount >= load.amount
            }) else {
                continue;
            };
            match *kind {
                NanobotType::Worker if has_source_stockpile => {
                    source_material = source_material.saturating_add(load.amount);
                }
                NanobotType::Hauler => {
                    let Some(assignment) =
                        assignment.filter(|assignment| assignment.source == reservation.source)
                    else {
                        continue;
                    };
                    let Ok((_, stockpile, _, role, owner, condition)) =
                        material_stockpiles.get(assignment.source)
                    else {
                        continue;
                    };
                    if stockpile.kind != load.kind
                        || !stockpile_belongs_to_swarm(owner)
                        || condition.is_some_and(|condition| !condition.is_operational())
                    {
                        continue;
                    }
                    match role.copied().unwrap_or_default() {
                        StockpileRole::Source => {
                            source_material = source_material.saturating_add(load.amount);
                        }
                        StockpileRole::Sink => {
                            sink_material = sink_material.saturating_add(load.amount);
                        }
                    }
                }
                _ => {}
            }
        }
        let has_sink_path = has_sink_stockpile
            || viable_planned_sink
            || local_consumer_sink_path
            || future_facility_sink_path;
        let deliverable_material =
            sink_material.saturating_add(if has_sink_path { source_material } else { 0 });
        let has_material_path =
            deliverable_material >= PRODUCTION_COST_PER_BOT || (gather_path && has_sink_path);
        let existing_facility_material_path =
            facilities.iter().any(|(facility, owner, _, condition)| {
                owner.is_some_and(|owner| owner.0 == swarm_entity)
                    && condition.is_none_or(|condition| condition.health > 0)
                    && ((gather_path && has_sink_path)
                        || facility.input_amount.saturating_add(deliverable_material)
                            >= PRODUCTION_COST_PER_BOT)
            });

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
            existing_facility_material_path,
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
        app.init_resource::<ProductionCollapseState>()
            .init_resource::<MatchOutcome>()
            .add_systems(
                FixedUpdate,
                production_collapse_detection_system
                    .after(crate::nanobot::production::production_facility_work_system)
                    .after(crate::nanobot::gather::source_stockpile_demand_system)
                    .after(crate::nanobot::planned::sink_stockpile_demand_system)
                    .after(crate::nanobot::RegionalAllocationSet::Project),
            )
            .add_systems(
                FixedUpdate,
                match_outcome_latch_system.after(production_collapse_detection_system),
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
