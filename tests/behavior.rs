#![allow(clippy::duplicate_mod)]

#[path = "behavior/build_zone.rs"]
mod build_zone;
#[path = "behavior/charger.rs"]
mod charger;
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
#[path = "behavior/sink_stockpile.rs"]
mod sink_stockpile;
#[path = "behavior/source_stockpile_flow.rs"]
mod source_stockpile_flow;
#[path = "behavior/source_stockpile_placement.rs"]
mod source_stockpile_placement;
#[path = "behavior/stockpile_and_haul.rs"]
mod stockpile_and_haul;
#[path = "behavior/zone_brush_ui_capture.rs"]
mod zone_brush_ui_capture;
#[path = "behavior/zone_overlay_draw_order.rs"]
mod zone_overlay_draw_order;
