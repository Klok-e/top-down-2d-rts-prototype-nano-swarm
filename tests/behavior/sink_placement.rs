use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        OwnerSwarm, PRODUCTION_COST_PER_BOT, PlannedKind, PlannedStructure, PlannedStructurePlugin,
        PopulationDemandPlugin, ProductionFacility, ProductionPlugin, SwarmId,
    },
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn occupied_facility_build_cell_can_plan_its_local_sink() {
    let mut app = common::sim_app();
    app.add_plugins((
        PlannedStructurePlugin,
        PopulationDemandPlugin,
        ProductionPlugin,
    ));
    let cell = IVec2::ZERO;
    let facility_pos = common::cell_world_center(cell);
    let player = common::spawn_swarm_at(&mut app, facility_pos);
    common::spawn_worker_at(&mut app, facility_pos + Vec2::new(-144.0, -144.0));
    common::spawn_hauler_at(&mut app, facility_pos + Vec2::new(-144.0, 0.0));
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, SwarmId::PLAYER);
    app.world_mut().spawn((
        ProductionFacility::new(),
        OwnerSwarm(player),
        Transform::from_translation(facility_pos.extend(0.0)),
    ));
    let source = common::spawn_stockpile(
        &mut app,
        common::cell_world_center(IVec2::new(-2, 0)),
        PRODUCTION_COST_PER_BOT,
        100,
    );
    app.world_mut()
        .entity_mut(source)
        .insert(OwnerSwarm(player));

    for _ in 0..100 {
        app.update();
        if app
            .world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .any(|plan| plan.kind == PlannedKind::SinkStockpile)
        {
            break;
        }
    }

    assert!(
        app.world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .any(|planned| planned.kind == PlannedKind::SinkStockpile),
        "the Sink planner can place beside a facility in its occupied Build cell",
    );
}
