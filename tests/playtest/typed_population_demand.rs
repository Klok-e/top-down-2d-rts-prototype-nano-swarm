//! Runtime regression for typed workload production.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        NanobotType, PRODUCTION_TICKS_PER_BOT, PopulationDemandPlugin, ProductionFacility,
        ProductionPriority, SwarmId, SwarmMember,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn defend_paint_eventually_produces_defender_despite_excess_haulers() {
    let mut app = common::sim_app_with_production();
    app.add_plugins(PopulationDemandPlugin);
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    for _ in 0..4 {
        common::spawn_worker_at(&mut app, Vec2::ZERO);
    }
    for _ in 0..10 {
        common::spawn_hauler_at(&mut app, Vec2::ZERO);
    }
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 25);
    priority.set_weight(NanobotType::Hauler, 60);
    priority.set_weight(NanobotType::Defender, 15);
    app.insert_resource(priority);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::ZERO,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::ZERO);
    common::fill_facility_input(&mut app, facility);

    app.update();
    assert_eq!(
        app.world()
            .entity(facility)
            .get::<ProductionFacility>()
            .unwrap()
            .current_target,
        Some(NanobotType::Defender),
    );

    for _ in 0..PRODUCTION_TICKS_PER_BOT {
        app.update();
    }

    let world = app.world_mut();
    let defenders = world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, member)| **kind == NanobotType::Defender && member.0 == SwarmId::PLAYER)
        .count();
    assert_eq!(defenders, 1);
}
