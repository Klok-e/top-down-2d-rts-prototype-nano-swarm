#![allow(clippy::duplicate_mod)]

#[path = "behavior/charger.rs"]
mod charger;
#[path = "behavior/charger_planned.rs"]
mod charger_planned;
#[path = "behavior/defend_zone.rs"]
mod defend_zone;
#[path = "behavior/full_source_stockpile.rs"]
mod full_source_stockpile;
#[path = "behavior/gather_overlap.rs"]
mod gather_overlap;
#[path = "behavior/gather_zone.rs"]
mod gather_zone;
#[path = "behavior/hauler_corridor.rs"]
mod hauler_corridor;
#[path = "behavior/intent_brush.rs"]
mod intent_brush;
#[path = "behavior/maintenance.rs"]
mod maintenance;
#[path = "behavior/nanobot_autonomy.rs"]
mod nanobot_autonomy;
#[path = "behavior/no_instant_spawning.rs"]
mod no_instant_spawning;
#[path = "behavior/opponent_swarm.rs"]
mod opponent_swarm;
#[path = "behavior/per_swarm_intent_ownership.rs"]
mod per_swarm_intent_ownership;
#[path = "behavior/planned_structure.rs"]
mod planned_structure;
#[path = "behavior/production_collapse.rs"]
mod production_collapse;
#[path = "behavior/production_facility.rs"]
mod production_facility;
#[path = "behavior/production_facility_planned.rs"]
mod production_facility_planned;
#[path = "behavior/production_ratio_panel.rs"]
mod production_ratio_panel;
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
#[path = "behavior/zone_brush_ui_capture.rs"]
mod zone_brush_ui_capture;
#[path = "behavior/zone_overlay_draw_order.rs"]
mod zone_overlay_draw_order;
