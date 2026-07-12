//! Focused behavior coverage for gradual, ledger-neutral hauler pickup.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, Charger, DirectMovementComponent, HaulerAssignment, LogisticsReservation,
        OwnerSwarm, ProductionFacility, SwarmId, HAULER_EXTRACT_PER_TICK,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn hauler_reserves_partial_terminal_load_then_picks_it_up_gradually() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let source_amount = 13;
    let source = common::spawn_sink_stockpile(&mut app, source_pos, source_amount, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let terminal = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_translation(Vec2::new(200.0, 0.0).extend(0.0)),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .resource_mut::<ResourceLedger>()
        .add(ResourceKind::Minerals, source_amount);

    app.update();

    let reservation = *app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("the assigned hauler reserves its exact partial load");
    assert_eq!(reservation.source, source);
    assert_eq!(reservation.destination, terminal);
    assert_eq!(reservation.amount, source_amount);
    assert_eq!(reservation.kind, ResourceKind::Minerals);
    assert_eq!(
        app.world()
            .entity(source)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        source_amount,
        "assignment and reservation do not move minerals",
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        source_amount,
        "reservation does not change owning swarm minerals",
    );
    assert!(
        app.world().entity(hauler).get::<Cargo>().is_none(),
        "cargo appears only when physical pickup starts",
    );

    app.world_mut()
        .entity_mut(hauler)
        .insert(Transform::from_translation(source_pos.extend(0.0)))
        .remove::<DirectMovementComponent>();
    app.update();

    let stockpile_amount = app
        .world()
        .entity(source)
        .get::<Stockpile>()
        .unwrap()
        .amount;
    let cargo = app
        .world()
        .entity(hauler)
        .get::<Cargo>()
        .expect("loading creates explicit hauler cargo");
    assert_eq!(cargo.kind, ResourceKind::Minerals);
    assert_eq!(cargo.amount, HAULER_EXTRACT_PER_TICK);
    assert_eq!(stockpile_amount, source_amount - HAULER_EXTRACT_PER_TICK);
    let remaining_claim = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("reservation remains throughout gradual loading");
    assert_eq!(
        remaining_claim.source_remaining,
        source_amount - HAULER_EXTRACT_PER_TICK,
        "physical pickup releases only the transferred source claim",
    );
    assert_eq!(
        remaining_claim.destination_remaining, source_amount,
        "destination claim remains intact until unloading",
    );
    assert_eq!(
        stockpile_amount + cargo.amount,
        source_amount,
        "one loading tick moves equal amounts out of the stockpile and into cargo",
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        source_amount,
        "pickup keeps cargo in owning swarm ledger total",
    );
}

#[test]
fn same_tick_haulers_reserve_only_available_source_and_destination() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(100.0, 0.0), 25, 100);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(200.0, 0.0), 0, 25);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let first = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let second = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    let reservations = [first, second]
        .into_iter()
        .filter_map(|hauler| {
            app.world()
                .entity(hauler)
                .get::<LogisticsReservation>()
                .copied()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reservations.len(),
        2,
        "both haulers can claim a partial trip"
    );
    assert_eq!(
        reservations.iter().map(|claim| claim.amount).sum::<u32>(),
        25,
        "same-tick claims cannot exceed source amount or destination capacity",
    );
}

#[test]
fn hauler_unloads_into_stockpile_gradually_and_ledger_neutrally() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 100);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 3, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        HaulerAssignment { source, sink },
        reservation,
    ));
    app.world_mut()
        .resource_mut::<ResourceLedger>()
        .add(ResourceKind::Minerals, 13);

    app.update();

    assert_eq!(
        app.world().entity(sink).get::<Stockpile>().unwrap().amount,
        7
    );
    assert_eq!(app.world().entity(hauler).get::<Cargo>().unwrap().amount, 6);
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.source_remaining, 0);
    assert_eq!(reservation.destination_remaining, 6);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total(ResourceKind::Minerals),
        13,
        "unloading changes location, not swarm ownership",
    );
}

#[test]
fn hauler_unloads_into_facility_gradually() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let sink = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::default(),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        HaulerAssignment { source, sink },
        reservation,
    ));

    app.update();

    assert_eq!(
        app.world()
            .entity(sink)
            .get::<ProductionFacility>()
            .unwrap()
            .input_amount,
        HAULER_EXTRACT_PER_TICK,
    );
    assert_eq!(app.world().entity(hauler).get::<Cargo>().unwrap().amount, 6);
}

#[test]
fn hauler_unloads_into_charger_gradually() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let mut charger = Charger::new(IVec2::ZERO);
    charger.amount = 0;
    let sink = app
        .world_mut()
        .spawn((charger, OwnerSwarm(swarm), Transform::default()))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        HaulerAssignment { source, sink },
        reservation,
    ));

    app.update();

    assert_eq!(
        app.world().entity(sink).get::<Charger>().unwrap().amount,
        HAULER_EXTRACT_PER_TICK,
    );
    assert_eq!(app.world().entity(hauler).get::<Cargo>().unwrap().amount, 6);
}

#[test]
fn loaded_hauler_from_source_reroutes_only_to_same_swarm_sink() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(-100.0, 0.0), 0, 100);
    let full_sink = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 100, 100);
    let replacement_pos = Vec2::new(250.0, 0.0);
    let replacement = common::spawn_sink_stockpile(&mut app, replacement_pos, 0, 100);
    for stockpile in [source, full_sink, replacement] {
        app.world_mut()
            .entity_mut(stockpile)
            .insert(OwnerSwarm(swarm));
    }
    let terminal = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_translation(Vec2::new(200.0, 0.0).extend(0.0)),
        ))
        .id();
    let enemy_swarm = app.world_mut().spawn(SwarmId(99)).id();
    app.world_mut().spawn((
        ProductionFacility::new(),
        OwnerSwarm(enemy_swarm),
        Transform::from_translation(Vec2::new(50.0, 0.0).extend(0.0)),
    ));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut reservation = LogisticsReservation::new(source, full_sink, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        HaulerAssignment {
            source,
            sink: full_sink,
        },
        reservation,
    ));

    app.update();

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("loaded cargo keeps physical transport assignment");
    assert_eq!(assignment.sink, replacement);
    assert_eq!(
        app.world().entity(hauler).get::<Cargo>().unwrap().amount,
        10
    );
    assert_eq!(
        app.world()
            .entity(terminal)
            .get::<ProductionFacility>()
            .unwrap()
            .input_amount,
        0,
        "Source-tier cargo cannot bypass Sink stockpiles for a terminal",
    );
    let movement = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("hauler physically travels to replacement Sink stockpile");
    assert_eq!(movement.xy, replacement_pos);
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.destination, replacement);
    assert_eq!(reservation.destination_remaining, 10);
}

#[test]
fn loaded_hauler_returns_to_source_when_destination_is_missing_and_no_terminal_exists() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(-100.0, 0.0);
    let source = common::spawn_sink_stockpile(&mut app, source_pos, 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let missing = app.world_mut().spawn_empty().id();
    app.world_mut().despawn(missing);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut reservation = LogisticsReservation::new(source, missing, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        HaulerAssignment {
            source,
            sink: missing,
        },
        reservation,
    ));

    app.update();

    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .unwrap()
            .sink,
        source,
    );
    assert_eq!(
        app.world().entity(hauler).get::<Cargo>().unwrap().amount,
        10
    );
    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<DirectMovementComponent>()
            .unwrap()
            .xy,
        source_pos,
        "fallback remains a physical return trip",
    );
}

#[test]
fn owned_hauler_waits_and_releases_claim_when_only_foreign_or_unowned_destinations_exist() {
    let mut app = common::sim_app_with_gather_haul();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app.world_mut().spawn(SwarmId(9)).id();
    let source = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 0, 0);
    let foreign = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 0, 100);
    let unowned = common::spawn_sink_stockpile(&mut app, Vec2::new(50.0, 0.0), 0, 100);
    app.world_mut()
        .entity_mut(source)
        .insert(OwnerSwarm(player));
    app.world_mut()
        .entity_mut(foreign)
        .insert(OwnerSwarm(enemy));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    let mut reservation = LogisticsReservation::new(source, foreign, ResourceKind::Minerals, 10);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 10,
        },
        HaulerAssignment {
            source,
            sink: foreign,
        },
        reservation,
    ));

    app.update();

    assert_eq!(
        app.world()
            .entity(foreign)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        0
    );
    assert_eq!(
        app.world()
            .entity(unowned)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        0
    );
    assert_eq!(
        app.world().entity(hauler).get::<Cargo>().unwrap().amount,
        10
    );
    assert!(app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .is_none());
    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<LogisticsReservation>()
            .unwrap()
            .destination_remaining,
        0,
    );
}

#[test]
fn same_tick_reroutes_cannot_overbook_replacement_capacity() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 0, 0);
    let full = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 100, 100);
    let replacement = common::spawn_sink_stockpile(&mut app, Vec2::new(100.0, 0.0), 0, 15);
    for stockpile in [source, full, replacement] {
        app.world_mut()
            .entity_mut(stockpile)
            .insert(OwnerSwarm(swarm));
    }
    let haulers = [
        common::spawn_hauler_at(&mut app, Vec2::ZERO),
        common::spawn_hauler_at(&mut app, Vec2::ZERO),
    ];
    for hauler in haulers {
        let mut reservation = LogisticsReservation::new(source, full, ResourceKind::Minerals, 10);
        reservation.source_remaining = 0;
        app.world_mut().entity_mut(hauler).insert((
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 10,
            },
            HaulerAssignment { source, sink: full },
            reservation,
        ));
    }

    app.update();

    let claims = haulers
        .into_iter()
        .map(|hauler| {
            *app.world()
                .entity(hauler)
                .get::<LogisticsReservation>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.destination == replacement)
            .map(|claim| claim.destination_remaining)
            .sum::<u32>(),
        10,
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| claim.destination != replacement)
            .map(|claim| claim.destination_remaining)
            .sum::<u32>(),
        0,
        "hauler with no replacement releases old destination claim",
    );
}
