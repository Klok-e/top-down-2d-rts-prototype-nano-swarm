//! Runtime regression for typed workload production.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        NanobotType, PRODUCTION_TICKS_PER_BOT, PopulationDemand, PopulationDemandPlugin,
        ProductionFacility, ProductionPriority, Swarm, SwarmId, SwarmMember,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn swarm_tile_reserve_eventually_produces_defender_despite_excess_haulers() {
    let mut app = common::sim_app_with_production();
    app.add_plugins(PopulationDemandPlugin);
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    for _ in 0..4 {
        common::spawn_worker_at(&mut app, Vec2::ZERO);
    }
    for _ in 0..10 {
        common::spawn_hauler_at(&mut app, Vec2::ZERO);
    }
    common::spawn_defender_at(&mut app, Vec2::ZERO);
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 25);
    priority.set_weight(NanobotType::Hauler, 60);
    priority.set_weight(NanobotType::Defender, 15);
    app.insert_resource(priority);
    for x in 0..4 {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            IVec2::new(x, 0),
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
    }
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::ZERO);
    common::fill_facility_input(&mut app, facility);

    app.update();
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Defender),
        2,
        "four Defend cells are four ordinary Swarm Tiles, not four Defender slots",
    );
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
    assert_eq!(defenders, 2);
}

#[test]
fn physical_threats_drive_eventual_defender_production_above_reserve() {
    let mut app = common::sim_app_with_production();
    app.add_plugins(PopulationDemandPlugin);
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Defender, 1);
    app.insert_resource(priority);
    let player_swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_defender_at(&mut app, Vec2::ZERO);
    let opponent_id = SwarmId(7);
    app.world_mut().spawn((Swarm {}, opponent_id));
    for _ in 0..3 {
        let hostile = common::spawn_worker_at(&mut app, Vec2::new(64.0, 64.0));
        app.world_mut()
            .entity_mut(hostile)
            .insert(SwarmMember::new(opponent_id));
    }
    for x in 0..2 {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            IVec2::new(x, 0),
            IntentKind::Gather,
            Some(SwarmId::PLAYER),
        );
    }
    let facilities = [
        common::spawn_facility_at(&mut app, player_swarm, Vec2::ZERO),
        common::spawn_facility_at(&mut app, player_swarm, Vec2::new(64.0, 0.0)),
    ];
    for facility in facilities {
        common::fill_facility_input(&mut app, facility);
    }

    app.update();

    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Defender),
        3,
        "three hostile nanobots on territory exceed the two-tile reserve of one",
    );
    for facility in facilities {
        assert_eq!(
            app.world()
                .entity(facility)
                .get::<ProductionFacility>()
                .unwrap()
                .current_target,
            Some(NanobotType::Defender),
        );
    }

    for _ in 0..PRODUCTION_TICKS_PER_BOT {
        app.update();
    }

    let world = app.world_mut();
    let defenders = world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, member)| **kind == NanobotType::Defender && member.0 == SwarmId::PLAYER)
        .count();
    assert_eq!(defenders, 3);
}
