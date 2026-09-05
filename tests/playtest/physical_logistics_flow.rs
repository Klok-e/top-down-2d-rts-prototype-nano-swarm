//! Scripted near-runtime physical logistics custody flow.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, DirectMovementComponent, HAULER_EXTRACT_PER_TICK, HAULER_TRANSFER_PER_TICK,
        HaulerAssignment, HaulerLoading, LogisticsReservation, OwnerSwarm, ProductionFacility,
        SwarmId,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
};

#[path = "../common/mod.rs"]
mod common;

const INITIAL_MINERALS: u32 = 20;

#[test]
fn assignment_load_transit_and_terminal_unload_preserve_physical_custody() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source_pos = Vec2::new(100.0, 0.0);
    let terminal_pos = Vec2::new(300.0, 0.0);
    let source = common::spawn_sink_stockpile(&mut app, source_pos, INITIAL_MINERALS, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let terminal = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_translation(terminal_pos.extend(0.0)),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        INITIAL_MINERALS,
    );

    for _ in 0..80 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("runtime allocator assigns source to terminal");
    assert_eq!((assignment.source, assignment.sink), (source, terminal));
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("assignment creates exact logistics reservation");
    assert_eq!(reservation.amount, INITIAL_MINERALS);
    assert_eq!(reservation.source_remaining, INITIAL_MINERALS);
    assert_eq!(reservation.destination_remaining, INITIAL_MINERALS);
    assert!(app.world().entity(hauler).get::<Cargo>().is_none());
    assert_custody_and_ledger(&app, source, hauler, terminal);

    app.world_mut()
        .entity_mut(hauler)
        .insert(Transform::from_xyz(168.0, 0.0, 0.0))
        .remove::<DirectMovementComponent>();
    app.update();

    assert!(app.world().entity(hauler).contains::<HaulerLoading>());
    assert_eq!(cargo_amount(&app, hauler), HAULER_EXTRACT_PER_TICK);
    assert_eq!(
        stockpile_amount(&app, source),
        INITIAL_MINERALS - HAULER_EXTRACT_PER_TICK
    );
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(
        reservation.source_remaining,
        INITIAL_MINERALS - HAULER_EXTRACT_PER_TICK
    );
    assert_eq!(reservation.destination_remaining, INITIAL_MINERALS);
    assert_custody_and_ledger(&app, source, hauler, terminal);

    for _ in 0..16 {
        app.update();
        if !app.world().entity(hauler).contains::<HaulerLoading>() {
            break;
        }
    }
    app.update();
    assert_eq!(stockpile_amount(&app, source), 0);
    assert_eq!(cargo_amount(&app, hauler), INITIAL_MINERALS);
    assert!(!app.world().entity(hauler).contains::<HaulerLoading>());
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.source_remaining, 0);
    assert_eq!(reservation.destination_remaining, INITIAL_MINERALS);
    assert_custody_and_ledger(&app, source, hauler, terminal);

    let movement = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("runtime carry assignment starts terminal transit");
    assert!(movement.xy.distance(Vec2::new(232.0, 0.0)) < 0.001);
    app.update();
    let transit_pos = app
        .world()
        .entity(hauler)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    assert!(
        transit_pos.x > source_pos.x && transit_pos.x < terminal_pos.x,
        "loaded hauler physically advances during cargo transit"
    );
    assert_eq!(cargo_amount(&app, hauler), INITIAL_MINERALS);
    assert_custody_and_ledger(&app, source, hauler, terminal);

    app.world_mut()
        .entity_mut(hauler)
        .insert(Transform::from_xyz(232.0, 0.0, 0.0))
        .remove::<DirectMovementComponent>();
    app.update();

    assert_eq!(facility_amount(&app, terminal), HAULER_TRANSFER_PER_TICK);
    assert_eq!(
        cargo_amount(&app, hauler),
        INITIAL_MINERALS - HAULER_TRANSFER_PER_TICK
    );
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.source_remaining, 0);
    assert_eq!(
        reservation.destination_remaining,
        INITIAL_MINERALS - HAULER_TRANSFER_PER_TICK
    );
    assert_custody_and_ledger(&app, source, hauler, terminal);

    for _ in 0..16 {
        app.update();
        if app.world().entity(hauler).get::<Cargo>().is_none() {
            break;
        }
    }
    assert_eq!(stockpile_amount(&app, source), 0);
    assert_eq!(facility_amount(&app, terminal), INITIAL_MINERALS);
    assert!(app.world().entity(hauler).get::<Cargo>().is_none());
    assert!(
        app.world()
            .entity(hauler)
            .get::<LogisticsReservation>()
            .is_none()
    );
    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_none()
    );
    assert_custody_and_ledger(&app, source, hauler, terminal);
}

fn stockpile_amount(app: &App, entity: Entity) -> u32 {
    app.world()
        .entity(entity)
        .get::<Stockpile>()
        .unwrap()
        .amount
}

fn cargo_amount(app: &App, entity: Entity) -> u32 {
    app.world()
        .entity(entity)
        .get::<Cargo>()
        .map_or(0, |cargo| cargo.amount)
}

fn facility_amount(app: &App, entity: Entity) -> u32 {
    app.world()
        .entity(entity)
        .get::<ProductionFacility>()
        .unwrap()
        .input_amount
}

fn assert_custody_and_ledger(app: &App, source: Entity, hauler: Entity, terminal: Entity) {
    let physical =
        stockpile_amount(app, source) + cargo_amount(app, hauler) + facility_amount(app, terminal);
    assert_eq!(
        physical, INITIAL_MINERALS,
        "physical custody remains conserved"
    );
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        INITIAL_MINERALS,
        "transfers preserve owning swarm ledger"
    );
}

#[test]
fn blocked_delivery_returns_cargo_to_source_stockpile_physically() {
    use top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid;
    let mut app = common::sim_app_with_gather_haul();
    app.world_mut().insert_resource(IntentGrid::new(2, 2));
    let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(-350.0, 0.0), 0, 100);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(350.0, 0.0), 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(owner));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(owner));
    let wall = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 0);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    let bot = common::spawn_hauler_at(&mut app, Vec2::new(-180.0, 0.0));
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 12);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 12,
        },
        HaulerAssignment { source, sink },
        reservation,
    ));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        12,
    );
    for _ in 0..100 {
        app.update();
        let carried = app
            .world()
            .get::<Cargo>(bot)
            .map_or(0, |cargo| cargo.amount);
        assert_eq!(
            carried + stockpile_amount(&app, source) + stockpile_amount(&app, sink),
            12
        );
        assert_eq!(stockpile_amount(&app, sink), 0);
        assert!(app.world().get::<Transform>(bot).unwrap().translation.x < -65.9);
        if carried == 0 {
            break;
        }
    }
    assert_eq!(
        stockpile_amount(&app, source),
        12,
        "blocked delivery must physically return to a reachable compatible source"
    );
    assert!(app.world().get::<LogisticsReservation>(bot).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        12
    );
}

#[test]
fn blocked_delivery_waits_with_cargo_then_reopens_without_duplicate_transfer() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::IntentGrid, navigation_runtime::NavigationBudget,
    };
    let mut app = common::sim_app_with_gather_haul();
    app.world_mut().insert_resource(IntentGrid::new(2, 2));
    let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(-350.0, 0.0), 0, 0);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(350.0, 0.0), 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(owner));
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(owner));
    let wall = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 0);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    let bot = common::spawn_hauler_at(&mut app, Vec2::new(-180.0, 0.0));
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 12);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 12,
        },
        HaulerAssignment { source, sink },
        reservation,
    ));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        12,
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<LogisticsReservation>(bot)
            .unwrap()
            .destination_remaining,
        12,
        "pending access preserves the commitment"
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32768;
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(app.world().get::<Cargo>(bot).unwrap().amount, 12);
    assert_eq!(
        app.world()
            .get::<LogisticsReservation>(bot)
            .unwrap()
            .destination_remaining,
        0,
        "proven blocked destination releases its capacity"
    );
    assert_eq!(stockpile_amount(&app, sink), 0);
    assert!(app.world().get::<Transform>(bot).unwrap().translation.x < -65.9);
    app.world_mut().despawn(wall);
    for _ in 0..180 {
        app.update();
        assert_eq!(
            app.world().get::<Cargo>(bot).map_or(0, |c| c.amount) + stockpile_amount(&app, sink),
            12
        );
    }
    assert_eq!(stockpile_amount(&app, sink), 12);
    assert!(app.world().get::<Cargo>(bot).is_none());
    assert!(app.world().get::<LogisticsReservation>(bot).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        12
    );
}

#[test]
fn blocked_delivery_prefers_another_sink_before_returning_to_source_for_both_swarms() {
    use top_down_2d_rts_prototype_nano_swarm::{intent::IntentGrid, nanobot::SwarmMember};
    for swarm in [SwarmId::PLAYER, SwarmId(7)] {
        let mut app = common::sim_app_with_gather_haul();
        app.world_mut().insert_resource(IntentGrid::new(2, 2));
        let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        app.world_mut().entity_mut(owner).insert(swarm);
        let source = common::spawn_stockpile(&mut app, Vec2::new(-350.0, 0.0), 0, 100);
        let blocked = common::spawn_sink_stockpile(&mut app, Vec2::new(350.0, 0.0), 0, 100);
        let alternate = common::spawn_sink_stockpile(&mut app, Vec2::new(-250.0, 250.0), 0, 100);
        for endpoint in [source, blocked, alternate] {
            app.world_mut()
                .entity_mut(endpoint)
                .insert(OwnerSwarm(owner));
        }
        let wall = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 0);
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
        let bot = common::spawn_hauler_at(&mut app, Vec2::new(-180.0, 0.0));
        let mut reservation =
            LogisticsReservation::new(source, blocked, ResourceKind::Minerals, 12);
        reservation.source_remaining = 0;
        app.world_mut().entity_mut(bot).insert((
            SwarmMember::new(swarm),
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 12,
            },
            HaulerAssignment {
                source,
                sink: blocked,
            },
            reservation,
        ));
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(swarm, ResourceKind::Minerals, 12);
        for _ in 0..150 {
            app.update();
            assert_eq!(
                stockpile_amount(&app, source),
                0,
                "reachable sink precedes physical return"
            );
            assert_eq!(stockpile_amount(&app, blocked), 0);
            assert_eq!(
                app.world().get::<Cargo>(bot).map_or(0, |c| c.amount)
                    + stockpile_amount(&app, alternate),
                12
            );
        }
        assert_eq!(stockpile_amount(&app, alternate), 12);
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(swarm, ResourceKind::Minerals),
            12
        );
    }
}

#[test]
fn interrupted_partial_pickup_delivers_only_the_cargo_already_loaded() {
    use top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid;
    let mut app = common::sim_app_with_gather_haul();
    app.world_mut().insert_resource(IntentGrid::new(2, 2));
    let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(350.0, 0.0), 16, 100);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(-350.0, 0.0), 0, 100);
    for endpoint in [source, sink] {
        app.world_mut()
            .entity_mut(endpoint)
            .insert(OwnerSwarm(owner));
    }
    let wall = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 0);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    let bot = common::spawn_hauler_at(&mut app, Vec2::new(-180.0, 0.0));
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 20);
    reservation.source_remaining = 16;
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
        HaulerAssignment { source, sink },
        HaulerLoading,
        reservation,
    ));
    for _ in 0..100 {
        app.update();
        assert_eq!(
            app.world().get::<Cargo>(bot).map_or(0, |c| c.amount) + stockpile_amount(&app, sink),
            4
        );
        assert_eq!(stockpile_amount(&app, source), 16);
    }
    assert_eq!(
        stockpile_amount(&app, sink),
        4,
        "failed remaining pickup must not strand a partial load"
    );
    assert!(app.world().get::<LogisticsReservation>(bot).is_none());
}

#[test]
fn terminal_delivery_tries_another_terminal_before_returning_to_pickup_sink() {
    use top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid;
    let mut app = common::sim_app_with_gather_haul();
    app.world_mut().insert_resource(IntentGrid::new(2, 2));
    let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-350.0, 0.0), 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(owner));
    let blocked = common::spawn_facility_at(&mut app, owner, Vec2::new(350.0, 0.0));
    let alternate = common::spawn_facility_at(&mut app, owner, Vec2::new(-250.0, 250.0));
    for facility in [blocked, alternate] {
        app.world_mut()
            .get_mut::<ProductionFacility>(facility)
            .unwrap()
            .input_amount = 0;
    }
    let wall = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 0);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    let bot = common::spawn_hauler_at(&mut app, Vec2::new(-180.0, 0.0));
    let mut reservation = LogisticsReservation::new(source, blocked, ResourceKind::Minerals, 12);
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(bot).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 12,
        },
        HaulerAssignment {
            source,
            sink: blocked,
        },
        reservation,
    ));
    for _ in 0..150 {
        app.update();
        assert_eq!(
            stockpile_amount(&app, source),
            0,
            "reachable alternate terminal precedes return to pickup Sink"
        );
        assert_eq!(facility_amount(&app, blocked), 0);
        assert_eq!(
            app.world().get::<Cargo>(bot).map_or(0, |c| c.amount)
                + facility_amount(&app, alternate),
            12
        );
        if facility_amount(&app, alternate) == 12 {
            break;
        }
    }
    assert_eq!(facility_amount(&app, alternate), 12);
}
