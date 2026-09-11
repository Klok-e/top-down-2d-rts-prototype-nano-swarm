#![allow(clippy::duplicate_mod)]

#[path = "behavior/battle_statistics.rs"]
mod battle_statistics;

#[path = "behavior/strategic_controller.rs"]
mod strategic_controller;

#[path = "behavior/gameplay_pacing.rs"]
mod gameplay_pacing;

#[path = "behavior/actionable_projection.rs"]
mod actionable_projection;
#[path = "behavior/automatic_construction_issue34.rs"]
mod automatic_construction_issue34;
#[path = "behavior/charger.rs"]
mod charger;
#[path = "behavior/charger_planned.rs"]
mod charger_planned;
#[path = "behavior/combat.rs"]
mod combat;
#[path = "behavior/combat_presentation.rs"]
mod combat_presentation;
#[path = "behavior/defender_response.rs"]
mod defender_response;
#[path = "behavior/defender_staging.rs"]
mod defender_staging;
#[path = "behavior/fixed_simulation.rs"]
mod fixed_simulation;
#[path = "behavior/full_source_stockpile.rs"]
mod full_source_stockpile;
#[path = "behavior/gather_overlap.rs"]
mod gather_overlap;
#[path = "behavior/gather_owner_filter.rs"]
mod gather_owner_filter;
#[path = "behavior/gather_zone.rs"]
mod gather_zone;
#[path = "behavior/gradual_hauler_pickup.rs"]
mod gradual_hauler_pickup;
#[path = "behavior/hauler_corridor.rs"]
mod hauler_corridor;
#[path = "behavior/idle_spread.rs"]
mod idle_spread;
#[path = "behavior/intent_brush.rs"]
mod intent_brush;
#[path = "behavior/maintenance.rs"]
mod maintenance;
#[path = "behavior/match_banner.rs"]
mod match_banner;
#[path = "behavior/movement.rs"]
mod movement;
#[path = "behavior/nanobot_presentation.rs"]
mod nanobot_presentation;
#[path = "behavior/no_instant_spawning.rs"]
mod no_instant_spawning;
#[path = "behavior/opponent_intent_controller.rs"]
mod opponent_intent_controller;
#[path = "behavior/opponent_swarm.rs"]
mod opponent_swarm;
#[path = "behavior/per_swarm_intent_ownership.rs"]
mod per_swarm_intent_ownership;
#[path = "behavior/physical_worker_gather.rs"]
mod physical_worker_gather;
#[path = "behavior/planned_structure.rs"]
mod planned_structure;
#[path = "behavior/population_demand.rs"]
mod population_demand;
#[path = "behavior/production_facility.rs"]
mod production_facility;
#[path = "behavior/production_facility_planned.rs"]
mod production_facility_planned;
#[path = "behavior/regional_allocation.rs"]
mod regional_allocation;
#[path = "behavior/sink_placement.rs"]
mod sink_placement;
#[path = "behavior/sink_stockpile.rs"]
mod sink_stockpile;
#[path = "behavior/source_stockpile_flow.rs"]
mod source_stockpile_flow;
#[path = "behavior/source_stockpile_placement.rs"]
mod source_stockpile_placement;
#[path = "behavior/stockpile_and_haul.rs"]
mod stockpile_and_haul;
#[path = "behavior/structure_overlay.rs"]
mod structure_overlay;
#[path = "behavior/tactical_overlay.rs"]
mod tactical_overlay;
#[path = "behavior/terminal_logistics_priority.rs"]
mod terminal_logistics_priority;
#[path = "behavior/territory_projection.rs"]
mod territory_projection;
#[path = "behavior/world_space_nanobots.rs"]
mod world_space_nanobots;
#[path = "behavior/zone_brush_ui_capture.rs"]
mod zone_brush_ui_capture;

#[path = "behavior/exterior_haul.rs"]
mod exterior_haul;
#[path = "behavior/exterior_movement.rs"]
mod exterior_movement;
#[path = "behavior/exterior_work.rs"]
mod exterior_work;
#[path = "behavior/local_avoidance.rs"]
mod local_avoidance;
#[path = "behavior/production_exits.rs"]
mod production_exits;

#[path = "behavior/construction_access.rs"]
mod construction_access;
#[path = "behavior/structure_clearing.rs"]
mod structure_clearing;

#[path = "behavior/navigation_budget.rs"]
mod navigation_budget;

#[path = "behavior/physical_world.rs"]
mod physical_world;

#[path = "behavior/task_reachability.rs"]
mod task_reachability;
#[path = "behavior/worker_route_recovery.rs"]
mod worker_route_recovery;

#[path = "behavior/independent_build_intent.rs"]
mod independent_build_intent;

#[path = "behavior/congestion.rs"]
mod congestion;

#[path = "behavior/work_navigation.rs"]
mod work_navigation;

#[path = "behavior/rock_terrain.rs"]
mod rock_terrain;

#[path = "behavior/deposit_presentation.rs"]
mod deposit_presentation;

#[path = "behavior/scenario_selection.rs"]
mod scenario_selection;

#[path = "behavior/swarm_elimination.rs"]
mod swarm_elimination;

#[path = "behavior/agent_match_snapshot.rs"]
mod agent_match_snapshot;
