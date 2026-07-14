//! Physical worker gather behavior.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, DirectMovementComponent, GatherAssignment, HAULER_TRANSFER_PER_TICK,
        LogisticsReservation, OwnerSwarm, ReturningToStockpile,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn worker_reserves_exact_partial_trip_without_moving_minerals() {
    let mut app = common::sim_app_with_gather();
    let pos = Vec2::ZERO;
    let swarm = common::spawn_swarm_at(&mut app, pos);
    let worker = common::spawn_worker_at(&mut app, pos);
    let deposit = common::spawn_deposit(&mut app, pos, 3);
    let stockpile = common::spawn_stockpile(&mut app, pos, 8, 10);
    app.world_mut()
        .entity_mut(stockpile)
        .insert((StockpileRole::Source, OwnerSwarm(swarm)));
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::ZERO, deposit));

    app.update();

    let reservation = app
        .world()
        .entity(worker)
        .get::<LogisticsReservation>()
        .expect("worker reserves source and destination before extraction");
    assert_eq!(reservation.source, deposit);
    assert_eq!(reservation.destination, stockpile);
    assert_eq!(reservation.kind, ResourceKind::Minerals);
    assert_eq!(
        reservation.amount, 2,
        "partial destination capacity limits trip"
    );
    assert_eq!(reservation.source_remaining, 2);
    assert_eq!(reservation.destination_remaining, 2);
    assert_eq!(
        app.world()
            .entity(deposit)
            .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
            .unwrap()
            .amount,
        3,
        "reservation does not remove deposit minerals",
    );
    assert_eq!(
        app.world()
            .entity(stockpile)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        8
    );
    assert_eq!(app.world().entity(worker).get::<Cargo>().unwrap().amount, 0);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total(ResourceKind::Minerals),
        0,
        "deposit remains outside swarm custody until extraction",
    );
}

#[test]
fn extraction_moves_only_new_minerals_into_cargo_and_ledger() {
    let mut app = common::sim_app_with_gather();
    let pos = Vec2::ZERO;
    let swarm = common::spawn_swarm_at(&mut app, pos);
    let worker = common::spawn_worker_at(&mut app, pos);
    let deposit = common::spawn_deposit(&mut app, pos, 10);
    let stockpile = common::spawn_stockpile(&mut app, pos, 0, 10);
    app.world_mut()
        .entity_mut(stockpile)
        .insert((StockpileRole::Source, OwnerSwarm(swarm)));
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::ZERO, deposit));

    app.update();
    app.update();

    assert_eq!(app.world().entity(worker).get::<Cargo>().unwrap().amount, 1);
    assert_eq!(
        app.world()
            .entity(deposit)
            .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
            .unwrap()
            .amount,
        9,
    );
    assert_eq!(
        app.world()
            .entity(stockpile)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        0
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total(ResourceKind::Minerals),
        1,
        "only physical deposit pickup enters swarm custody",
    );
    let reservation = app
        .world()
        .entity(worker)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.source_remaining, 3);
    assert_eq!(reservation.destination_remaining, 4);
}

#[test]
fn same_tick_workers_cannot_overbook_deposit_or_source_capacity() {
    let mut app = common::sim_app_with_gather();
    let pos = Vec2::ZERO;
    let swarm = common::spawn_swarm_at(&mut app, pos);
    let deposit = common::spawn_deposit(&mut app, pos, 5);
    let stockpile = common::spawn_stockpile(&mut app, pos, 0, 5);
    app.world_mut()
        .entity_mut(stockpile)
        .insert((StockpileRole::Source, OwnerSwarm(swarm)));
    let workers = [
        common::spawn_worker_at(&mut app, pos),
        common::spawn_worker_at(&mut app, pos),
    ];
    for worker in workers {
        app.world_mut()
            .entity_mut(worker)
            .insert(GatherAssignment::new(IVec2::ZERO, deposit));
    }

    app.update();

    let reservations = workers
        .into_iter()
        .map(|worker| {
            *app.world()
                .entity(worker)
                .get::<LogisticsReservation>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reservations.iter().map(|claim| claim.amount).sum::<u32>(),
        5
    );
    assert!(reservations.iter().all(|claim| claim.source == deposit));
    assert!(
        reservations
            .iter()
            .all(|claim| claim.destination == stockpile)
    );
    assert_eq!(
        app.world()
            .entity(deposit)
            .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
            .unwrap()
            .amount,
        5,
    );
    assert_eq!(
        app.world()
            .entity(stockpile)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        0
    );
}

#[test]
fn worker_unloads_gradually_at_shared_rate_without_changing_ledger() {
    let mut app = common::sim_app_with_gather();
    let pos = Vec2::ZERO;
    let swarm = common::spawn_swarm_at(&mut app, pos);
    let worker = common::spawn_worker_at(&mut app, pos);
    let source = common::spawn_deposit(&mut app, pos, 0);
    let stockpile = common::spawn_stockpile(&mut app, pos, 3, 100);
    app.world_mut()
        .entity_mut(stockpile)
        .insert((StockpileRole::Source, OwnerSwarm(swarm)));
    let mut reservation = LogisticsReservation::new(source, stockpile, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(worker).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        reservation,
        ReturningToStockpile { stockpile },
    ));
    app.world_mut()
        .resource_mut::<ResourceLedger>()
        .add(ResourceKind::Minerals, 13);

    app.update();

    assert_eq!(
        app.world()
            .entity(stockpile)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        3 + HAULER_TRANSFER_PER_TICK,
    );
    assert_eq!(
        app.world().entity(worker).get::<Cargo>().unwrap().amount,
        10 - HAULER_TRANSFER_PER_TICK,
    );
    assert_eq!(
        app.world()
            .entity(worker)
            .get::<LogisticsReservation>()
            .unwrap()
            .destination_remaining,
        10 - HAULER_TRANSFER_PER_TICK,
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total(ResourceKind::Minerals),
        13,
        "unloading changes location only",
    );
}

#[test]
fn loaded_worker_reroutes_from_wrong_owner_without_losing_cargo() {
    let mut app = common::sim_app_with_gather();
    let player_swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy_swarm = app
        .world_mut()
        .spawn((top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId(9),))
        .id();
    let worker = common::spawn_worker_at(&mut app, Vec2::ZERO);
    let source = common::spawn_deposit(&mut app, Vec2::ZERO, 0);
    let wrong = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 100);
    app.world_mut()
        .entity_mut(wrong)
        .insert((StockpileRole::Source, OwnerSwarm(enemy_swarm)));
    let replacement_pos = Vec2::new(100.0, 0.0);
    let replacement = common::spawn_stockpile(&mut app, replacement_pos, 0, 100);
    app.world_mut()
        .entity_mut(replacement)
        .insert((StockpileRole::Source, OwnerSwarm(player_swarm)));
    let mut reservation = LogisticsReservation::new(source, wrong, ResourceKind::Minerals, 4);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(worker).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        reservation,
        ReturningToStockpile { stockpile: wrong },
    ));

    app.update();

    assert_eq!(app.world().entity(worker).get::<Cargo>().unwrap().amount, 4);
    assert_eq!(
        app.world()
            .entity(worker)
            .get::<ReturningToStockpile>()
            .unwrap()
            .stockpile,
        replacement,
    );
    let reservation = app
        .world()
        .entity(worker)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.destination, replacement);
    assert_eq!(reservation.destination_remaining, 4);
    assert_eq!(
        app.world().entity(wrong).get::<Stockpile>().unwrap().amount,
        0
    );
}

#[test]
fn loaded_worker_waits_with_cargo_when_no_compatible_source_exists() {
    let mut app = common::sim_app_with_gather();
    let worker = common::spawn_worker_at(&mut app, Vec2::ZERO);
    let source = common::spawn_deposit(&mut app, Vec2::ZERO, 0);
    let missing = app.world_mut().spawn_empty().id();
    app.world_mut().despawn(missing);
    let mut reservation = LogisticsReservation::new(source, missing, ResourceKind::Minerals, 4);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(worker).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        reservation,
        ReturningToStockpile { stockpile: missing },
    ));

    app.update();

    assert_eq!(app.world().entity(worker).get::<Cargo>().unwrap().amount, 4);
    assert!(
        app.world()
            .entity(worker)
            .get::<ReturningToStockpile>()
            .is_none(),
        "worker releases invalid destination while waiting",
    );
    assert_eq!(
        app.world()
            .entity(worker)
            .get::<LogisticsReservation>()
            .unwrap()
            .destination_remaining,
        0,
        "unusable capacity claim is released",
    );
}

#[test]
fn loaded_worker_waits_without_movement_when_reserved_stockpile_is_full() {
    let mut app = common::sim_app_with_gather();
    let worker = common::spawn_worker_at(&mut app, Vec2::ZERO);
    let source = common::spawn_deposit(&mut app, Vec2::ZERO, 0);
    let full = common::spawn_stockpile(&mut app, Vec2::new(100.0, 0.0), 100, 100);
    let mut reservation = LogisticsReservation::new(source, full, ResourceKind::Minerals, 4);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(worker).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        reservation,
        ReturningToStockpile { stockpile: full },
    ));

    app.update();

    let worker_ref = app.world().entity(worker);
    assert_eq!(worker_ref.get::<Cargo>().unwrap().amount, 4);
    assert!(worker_ref.get::<ReturningToStockpile>().is_none());
    assert!(worker_ref.get::<DirectMovementComponent>().is_none());
    assert_eq!(
        worker_ref
            .get::<LogisticsReservation>()
            .unwrap()
            .destination_remaining,
        0,
    );
}
