use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{
    Charge, ChargerAssignment, DirectMovementComponent, HaulerAssignment, HaulerRoute,
    LogisticsReservation, OwnerSwarm, ProductionFacility, RegionalLease,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn empty_committed_defender_makes_charger_beat_startable_production() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(10.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));

    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(30.0, 0.0, 0.0),
        ))
        .id();
    let charger = common::spawn_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));

    let defender = common::spawn_defender_at(&mut app, Vec2::new(20.0, 0.0));
    app.world_mut().entity_mut(defender).insert((
        Charge {
            current: 0.0,
            max: 1.0,
        },
        ChargerAssignment { charger },
    ));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("hauler receives terminal work");
    assert_eq!(assignment.source, source);
    assert_eq!(assignment.sink, charger);
    assert_ne!(assignment.sink, facility);
}

#[test]
fn charger_emergency_reserves_only_uncovered_committed_charge_need() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(10.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let charger = common::spawn_charger_at(&mut app, IVec2::ZERO, 8);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));

    let defender = common::spawn_defender_at(&mut app, Vec2::new(20.0, 0.0));
    app.world_mut().entity_mut(defender).insert((
        Charge {
            current: 0.5,
            max: 1.0,
        },
        ChargerAssignment { charger },
    ));
    app.world_mut().spawn(LogisticsReservation::new(
        source,
        charger,
        top_down_2d_rts_prototype_nano_swarm::resources::ResourceKind::Minerals,
        1,
    ));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("partial emergency demand is reservable");
    assert_eq!(reservation.destination, charger);
    assert_eq!(reservation.amount, 3);
    assert_eq!(reservation.source_remaining, 3);
    assert_eq!(reservation.destination_remaining, 3);
}

#[test]
fn owned_terminal_ignores_shared_sink_stockpile() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let shared = common::spawn_sink_stockpile(&mut app, Vec2::new(1.0, 0.0), 100, 100);
    let owned = common::spawn_sink_stockpile(&mut app, Vec2::new(50.0, 0.0), 100, 100);
    app.world_mut().entity_mut(owned).insert(OwnerSwarm(swarm));
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(60.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("same-swarm sink supplies terminal");
    assert_eq!(assignment.source, owned);
    assert_ne!(assignment.source, shared);
    assert_eq!(assignment.sink, facility);
}

#[test]
fn waiting_production_eventually_beats_continuous_charger_emergency() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(100.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(120.0, 0.0, 0.0),
        ))
        .id();
    let charger = common::spawn_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, Vec2::ZERO);
    app.world_mut().entity_mut(defender).insert((
        Charge {
            current: 0.0,
            max: 1.0,
        },
        ChargerAssignment { charger },
    ));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    let mut production_selected = false;
    for _ in 0..32 {
        app.update();
        let sink = app
            .world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .map(|assignment| assignment.sink);
        if sink == Some(facility) {
            production_selected = true;
            break;
        }
        assert_eq!(sink, Some(charger));
        app.world_mut()
            .entity_mut(hauler)
            .remove::<HaulerAssignment>()
            .remove::<LogisticsReservation>()
            .remove::<HaulerRoute>()
            .remove::<DirectMovementComponent>()
            .remove::<RegionalLease>();
    }

    assert!(
        production_selected,
        "waiting-age fairness must prevent production starvation"
    );
}

#[test]
fn startable_production_beats_nearer_ordinary_charger_refill() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(10.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let charger = common::spawn_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let mut production = ProductionFacility::new();
    production.input_amount = 1;
    let facility = app
        .world_mut()
        .spawn((
            production,
            OwnerSwarm(swarm),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .unwrap()
            .sink,
        facility
    );
}

#[test]
fn larger_proportional_terminal_deficit_beats_shorter_route() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(10.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let near_half_empty = common::spawn_charger_at(&mut app, IVec2::ZERO, 30);
    app.world_mut()
        .entity_mut(near_half_empty)
        .insert(OwnerSwarm(swarm));
    let far_empty = common::spawn_charger_at(&mut app, IVec2::new(1, 0), 0);
    app.world_mut()
        .entity_mut(far_empty)
        .insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .unwrap()
            .sink,
        far_empty
    );
}

#[test]
fn route_cost_breaks_equal_terminal_demand_ties() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let near = common::spawn_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut().entity_mut(near).insert(OwnerSwarm(swarm));
    let far = common::spawn_charger_at(&mut app, IVec2::new(1, 0), 0);
    app.world_mut().entity_mut(far).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .unwrap()
            .sink,
        near
    );
}

#[test]
fn entity_id_breaks_fully_equal_terminal_ties() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let first = common::spawn_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut().entity_mut(first).insert(OwnerSwarm(swarm));
    let second = common::spawn_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut().entity_mut(second).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    let stable_min = if first.to_bits() < second.to_bits() {
        first
    } else {
        second
    };
    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .unwrap()
            .sink,
        stable_min
    );
}
