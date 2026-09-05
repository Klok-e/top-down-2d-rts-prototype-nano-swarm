use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, DirectMovementComponent, LogisticsReservation, ReturningToStockpile, SwarmId,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn worker_retains_cargo_and_releases_unreachable_capacity_then_resumes_after_obstacle_removal() {
    let mut app = common::sim_app_with_gather();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 100.0));
    let destination = common::spawn_stockpile(&mut app, Vec2::new(400.0, 100.0), 0, 10);
    let blocker = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(400.0, 100.0),
            amount: 0,
            capacity: 0,
            radius: 150.0,
        },
    );
    let mut reservation =
        LogisticsReservation::new(blocker, destination, ResourceKind::Minerals, 4);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(worker).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        reservation,
        ReturningToStockpile {
            stockpile: destination,
        },
    ));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        4,
    );
    for _ in 0..100 {
        app.update();
    }
    assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 4);
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 0);
    assert_eq!(
        app.world()
            .get::<LogisticsReservation>(worker)
            .unwrap()
            .destination_remaining,
        0
    );
    assert!(app.world().get::<ReturningToStockpile>(worker).is_none());
    assert!(app.world().get::<DirectMovementComponent>(worker).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        4
    );
    app.world_mut().despawn(blocker);
    for _ in 0..200 {
        app.update();
    }
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 4);
    assert!(app.world().get::<Cargo>(worker).is_none());
    assert!(app.world().get::<LogisticsReservation>(worker).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        4
    );
}

#[test]
fn worker_without_a_reservation_delivers_to_reachable_stockpile_instead_of_nearer_blocked_one() {
    let mut app = common::sim_app_with_gather();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 100.0));
    let blocked = common::spawn_stockpile(&mut app, Vec2::new(400.0, 100.0), 0, 10);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(400.0, 100.0),
            amount: 0,
            capacity: 0,
            radius: 150.0,
        },
    );
    let reachable = common::spawn_stockpile(&mut app, Vec2::new(100.0, 500.0), 0, 10);
    app.world_mut().entity_mut(worker).insert(Cargo {
        kind: ResourceKind::Minerals,
        amount: 4,
    });
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        4,
    );
    for _ in 0..200 {
        app.update();
    }
    assert_eq!(app.world().get::<Stockpile>(blocked).unwrap().amount, 0);
    assert_eq!(app.world().get::<Stockpile>(reachable).unwrap().amount, 4);
    assert!(app.world().get::<Cargo>(worker).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        4
    );
}

#[test]
fn pending_worker_destination_keeps_its_cargo_and_reservation_until_route_budget_recovers() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_gather();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 100.0));
    let destination = common::spawn_stockpile(&mut app, Vec2::new(400.0, 100.0), 0, 10);
    let mut reservation =
        LogisticsReservation::new(Entity::PLACEHOLDER, destination, ResourceKind::Minerals, 4);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(worker).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        reservation,
        ReturningToStockpile {
            stockpile: destination,
        },
    ));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        4,
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..12 {
        app.update();
    }
    assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 4);
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 0);
    assert_eq!(
        app.world()
            .get::<LogisticsReservation>(worker)
            .unwrap()
            .destination_remaining,
        4
    );
    assert_eq!(
        app.world()
            .get::<ReturningToStockpile>(worker)
            .unwrap()
            .stockpile,
        destination
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32_768;
    for _ in 0..160 {
        app.update();
    }
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 4);
    assert!(app.world().get::<Cargo>(worker).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        4
    );
}

#[test]
fn worker_does_not_extract_until_reserved_delivery_stockpile_is_reachable() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::GatherAssignment, resources::ResourceDeposit,
    };
    let mut app = common::sim_app_with_gather();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(168.0, 100.0));
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(100.0, 100.0),
            amount: 4,
            capacity: 4,
            radius: 32.0,
        },
    );
    let destination = common::spawn_stockpile(&mut app, Vec2::new(400.0, 100.0), 0, 10);
    let blocker = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(400.0, 100.0),
            amount: 0,
            capacity: 0,
            radius: 150.0,
        },
    );
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::ZERO, deposit));
    for _ in 0..100 {
        app.update();
    }
    assert_eq!(
        app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
        4
    );
    assert!(app.world().get::<LogisticsReservation>(worker).is_none());
    assert!(app.world().get::<Cargo>(worker).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        0
    );
    app.world_mut().despawn(blocker);
    for _ in 0..200 {
        app.update();
    }
    assert_eq!(
        app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
        0
    );
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 4);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        4
    );
}

#[test]
fn displaced_partial_worker_waits_for_source_route_then_delivers_cargo_when_source_is_blocked() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{ExtractProgress, GatherAssignment},
        navigation_runtime::NavigationBudget,
        resources::ResourceDeposit,
    };
    let mut app = common::sim_app_with_gather();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(168.0, 100.0));
    let source = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(100.0, 100.0),
            amount: 4,
            capacity: 4,
            radius: 32.0,
        },
    );
    let destination = common::spawn_stockpile(&mut app, Vec2::new(600.0, 100.0), 0, 10);
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::ZERO, source));
    for _ in 0..120 {
        app.update();
        if app
            .world()
            .get::<Cargo>(worker)
            .is_some_and(|cargo| cargo.amount == 1)
        {
            break;
        }
    }
    assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 1);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(100.0, 100.0),
            amount: 0,
            capacity: 0,
            radius: 150.0,
        },
    );
    app.world_mut()
        .get_mut::<Transform>(worker)
        .unwrap()
        .translation = Vec3::new(400.0, 100.0, 0.0);
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..12 {
        app.update();
    }
    assert!(app.world().get::<ExtractProgress>(worker).is_some());
    assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 1);
    assert_eq!(
        app.world()
            .get::<LogisticsReservation>(worker)
            .unwrap()
            .source_remaining,
        3
    );
    assert_eq!(
        app.world().get::<ResourceDeposit>(source).unwrap().amount,
        3
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32_768;
    for _ in 0..120 {
        app.update();
        if app.world().get::<ExtractProgress>(worker).is_none() {
            break;
        }
    }
    assert!(app.world().get::<ExtractProgress>(worker).is_none());
    assert_eq!(
        app.world()
            .get::<LogisticsReservation>(worker)
            .unwrap()
            .source_remaining,
        0
    );
    assert_eq!(app.world().get::<Cargo>(worker).unwrap().amount, 1);
    for _ in 0..120 {
        app.update();
    }
    assert_eq!(app.world().get::<Stockpile>(destination).unwrap().amount, 1);
    assert_eq!(
        app.world().get::<ResourceDeposit>(source).unwrap().amount,
        3
    );
    assert!(app.world().get::<Cargo>(worker).is_none());
    assert!(app.world().get::<LogisticsReservation>(worker).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        1
    );
}
