//! Scripted near-runtime physical logistics custody flow.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, DirectMovementComponent, HAULER_EXTRACT_PER_TICK, HAULER_TRANSFER_PER_TICK,
        HaulerAssignment, HaulerLoading, HaulerRoute, LogisticsReservation, OwnerSwarm,
        ProductionFacility, SwarmId,
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

    app.update();

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
        .insert(Transform::from_translation(source_pos.extend(0.0)))
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
    assert_eq!(movement.xy, terminal_pos);
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
        .insert(Transform::from_translation(terminal_pos.extend(0.0)))
        .remove::<DirectMovementComponent>()
        .remove::<HaulerRoute>();
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
