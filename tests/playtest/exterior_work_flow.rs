//! Complete physical work trips with independently measured body clearance.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{Cargo, OwnerSwarm, ProductionFacility},
    resources::{ResourceDeposit, Stockpile, StockpileRole},
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn exterior_gather_trip_extracts_and_delivers_at_scaled_target_surfaces() {
    let mut app = common::sim_app_with_gather();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId::PLAYER,
    );
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(140.0, 140.0),
            amount: 8,
            capacity: 8,
            radius: 48.0,
        },
    );
    app.world_mut().get_mut::<Transform>(deposit).unwrap().scale = Vec3::splat(1.5);
    let stockpile = common::spawn_stockpile(&mut app, Vec2::new(350.0, 140.0), 0, 100);
    app.world_mut()
        .entity_mut(stockpile)
        .insert(StockpileRole::Source);
    app.world_mut()
        .get_mut::<Transform>(stockpile)
        .unwrap()
        .scale = Vec3::new(2.0, 1.0, 1.0);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(30.0, 140.0));
    let mut remaining = 8;
    let mut delivered = 0;
    for _ in 0..500 {
        app.update();
        let position = app
            .world()
            .get::<Transform>(worker)
            .unwrap()
            .translation
            .truncate();
        let next_remaining = app.world().get::<ResourceDeposit>(deposit).unwrap().amount;
        let next_delivered = app.world().get::<Stockpile>(stockpile).unwrap().amount;
        if next_remaining < remaining {
            let surface_distance = position.distance(Vec2::new(140.0, 140.0)) - 48.0;
            assert!(
                (33.999..=38.001).contains(&surface_distance),
                "extraction must leave full 34-unit visual body exterior: {surface_distance}"
            );
        }
        if next_delivered > delivered {
            let offset = (position - Vec2::new(350.0, 140.0)).abs() - Vec2::new(64.0, 32.0);
            let surface_distance = offset.max(Vec2::ZERO).length();
            assert!(
                (33.999..=38.001).contains(&surface_distance),
                "delivery must leave full body exterior: {surface_distance}"
            );
        }
        remaining = next_remaining;
        delivered = next_delivered;
        if delivered == 8 {
            break;
        }
    }
    assert_eq!(
        remaining, 0,
        "both loads must be extracted by real movement"
    );
    assert_eq!(delivered, 8, "both loads must reach the scaled stockpile");
    assert!(app.world().get::<Cargo>(worker).is_none());
}

#[test]
fn exterior_hauler_trip_loads_and_unloads_without_center_teleports() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(150.0, 0.0), 20, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().get_mut::<Transform>(source).unwrap().scale = Vec3::new(2.0, 1.0, 1.0);
    let terminal = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(400.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut source_amount = 20;
    let mut terminal_amount = 0;
    for _ in 0..300 {
        app.update();
        let position = app
            .world()
            .get::<Transform>(hauler)
            .unwrap()
            .translation
            .truncate();
        let next_source = app.world().get::<Stockpile>(source).unwrap().amount;
        let next_terminal = app
            .world()
            .get::<ProductionFacility>(terminal)
            .unwrap()
            .input_amount;
        if next_source < source_amount {
            let surface_distance = ((position - Vec2::new(150.0, 0.0)).abs()
                - Vec2::new(64.0, 32.0))
            .max(Vec2::ZERO)
            .length();
            assert!(
                (33.999..=38.001).contains(&surface_distance),
                "loading clearance: {surface_distance}"
            );
        }
        if next_terminal > terminal_amount {
            let surface_distance = ((position - Vec2::new(400.0, 0.0)).abs() - Vec2::splat(32.0))
                .max(Vec2::ZERO)
                .length();
            assert!(
                (33.999..=38.001).contains(&surface_distance),
                "unloading clearance: {surface_distance}"
            );
        }
        source_amount = next_source;
        terminal_amount = next_terminal;
        if terminal_amount == 20 {
            break;
        }
    }
    assert_eq!(source_amount, 0);
    assert_eq!(
        terminal_amount, 20,
        "real transit must deliver the complete load"
    );
    assert!(app.world().get::<Cargo>(hauler).is_none());
}
