use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentGrid,
    nanobot::{
        AllocationClock, Charge, ChargerAssignment, DirectMovementComponent, HaulerAssignment,
        InteractionRegion, LogisticsReservation, NanobotType, OwnerSwarm, ProductionFacility,
        RegionalAllocationWake, RegionalLease, TerminalDemandAges,
    },
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
    navigation_runtime::NavigationBudget,
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn allocation_wakes_are_consumed_by_the_pass_that_serves_them() {
    let mut app = common::sim_app_with_gather_haul();
    let (_, terminal) = waiting_terminal(&mut app);
    let wake = |app: &mut App| {
        app.world_mut()
            .resource_mut::<RegionalAllocationWake>()
            .request_current_pass();
    };

    wake(&mut app);
    app.update();
    assert_eq!(waiting_age(&app, terminal), 1);
    app.update();
    assert_eq!(waiting_age(&app, terminal), 1, "initial pass consumed wake");

    wake(&mut app);
    app.update();
    assert_eq!(waiting_age(&app, terminal), 2, "standalone wake ran a pass");
    app.update();
    app.update();
    assert_eq!(waiting_age(&app, terminal), 2);
    assert_eq!(app.world().resource::<AllocationClock>().tick(), 0);

    wake(&mut app);
    app.update();
    assert_eq!(app.world().resource::<AllocationClock>().tick(), 1);
    assert_eq!(waiting_age(&app, terminal), 3);
    app.update();
    assert_eq!(
        waiting_age(&app, terminal),
        3,
        "scheduled pass consumed wake"
    );
}

#[test]
fn terminal_priority_stays_promoted_across_the_waiting_age_boundary() {
    for age_at_assignment in [2047, 2048] {
        let mut app = common::sim_app_with_gather_haul();
        let (source, oldest) = waiting_terminal(&mut app);
        for _ in 0..13_000 {
            if waiting_age(&app, oldest) == age_at_assignment - 1 {
                break;
            }
            app.update();
        }
        assert_eq!(waiting_age(&app, oldest), age_at_assignment - 1);

        let owner = *app.world().get::<OwnerSwarm>(source).unwrap();
        let mut facility = ProductionFacility::new();
        facility.input_amount = app
            .world()
            .resource::<top_down_2d_rts_prototype_nano_swarm::tuning::Tuning>()
            .production
            .cost_per_bot
            - 10;
        let newest = app
            .world_mut()
            .spawn((facility, owner, Transform::from_xyz(200.0, 120.0, 0.0)))
            .id();
        app.update();
        assert_eq!(waiting_age(&app, oldest), age_at_assignment - 1);

        let start = Vec2::new(-120.0, 0.0);
        let pickup = resolve_connection(&app, start, source);
        resolve_connection(&app, pickup, oldest);
        resolve_connection(&app, pickup, newest);
        let hauler = common::spawn_hauler_at(&mut app, start);
        for _ in 0..6 {
            app.update();
            if app.world().get::<HaulerAssignment>(hauler).is_some() {
                break;
            }
        }
        let assignment = app
            .world()
            .get::<HaulerAssignment>(hauler)
            .expect("the first allocation pass should assign a connected terminal");
        assert_eq!(
            assignment.sink, oldest,
            "the older terminal should still have priority at age {age_at_assignment}"
        );
    }
}

fn waiting_terminal(app: &mut App) -> (Entity, Entity) {
    let swarm = common::spawn_swarm_at(app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(app, Vec2::new(-200.0, 0.0), 10, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let terminal = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ))
        .id();
    (source, terminal)
}

fn waiting_age(app: &App, terminal: Entity) -> u32 {
    app.world()
        .resource::<TerminalDemandAges>()
        .waiting_ticks(terminal)
}

fn resolve_connection(app: &App, start: Vec2, target: Entity) -> Vec2 {
    let world = app.world();
    let goal = RouteGoal::Interaction(InteractionRegion::structure(
        world.get::<Transform>(target).unwrap(),
    ));
    let navigation = world.resource::<Navigation>();
    for _ in 0..32 {
        if let ConnectivityStatus::Connected { endpoint } =
            navigation.query_connectivity(start, goal)
        {
            return endpoint;
        }
        navigation.advance(
            world.resource::<IntentGrid>(),
            world.resource::<NavigationBudget>().0,
        );
    }
    panic!("the logistics connection should resolve")
}

#[test]
fn empty_committed_defender_makes_charger_beat_startable_production() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));

    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(700.0, 0.0, 0.0),
        ))
        .id();
    let charger = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
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
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

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
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let charger = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 80);
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
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .expect("partial emergency demand is reservable");
    assert_eq!(reservation.destination, charger);
    assert_eq!(reservation.amount, 9);
    assert_eq!(reservation.source_remaining, 9);
    assert_eq!(reservation.destination_remaining, 9);
}

#[test]
fn owned_terminal_ignores_shared_sink_stockpile() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let shared = common::spawn_sink_stockpile(&mut app, Vec2::new(-400.0, 0.0), 100, 100);
    let owned = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(owned).insert(OwnerSwarm(swarm));
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(700.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

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
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(700.0, 0.0, 0.0),
        ))
        .id();
    let charger = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
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
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    let mut production_selected = false;
    // The production terminal needs four eight-tick fairness promotions. At
    // the shipped 60 Hz simulation clock, the 10 Hz allocation clock reaches
    // that debt within four seconds.
    for _ in 0..240 {
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
        if sink.is_none() {
            continue;
        }
        assert_eq!(sink, Some(charger));
        app.world_mut()
            .entity_mut(hauler)
            .remove::<HaulerAssignment>()
            .remove::<LogisticsReservation>()
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
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let charger = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let mut production = ProductionFacility::new();
    production.input_amount = 20;
    let facility = app
        .world_mut()
        .spawn((
            production,
            OwnerSwarm(swarm),
            Transform::from_xyz(700.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

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
fn short_full_sink_refill_beats_long_trip_carrying_few_minerals() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let near_sink = common::spawn_sink_stockpile(&mut app, Vec2::new(0.0, 200.0), 0, 100);
    app.world_mut()
        .entity_mut(near_sink)
        .insert(OwnerSwarm(swarm));
    let far_sink = common::spawn_sink_stockpile(&mut app, Vec2::new(3000.0, 0.0), 100, 100);
    app.world_mut()
        .entity_mut(far_sink)
        .insert(OwnerSwarm(swarm));
    let mut production = ProductionFacility::new();
    production.input_amount = production.input_capacity - 2;
    app.world_mut().spawn((
        production,
        OwnerSwarm(swarm),
        Transform::from_xyz(3300.0, 0.0, 0.0),
    ));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-300.0, 0.0));

    wait_for_assignment(&mut app, hauler);

    assert_eq!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .unwrap()
            .sink,
        near_sink,
        "a startable cycle still loses when it costs far more per mineral",
    );
}

#[test]
fn route_cost_breaks_equal_terminal_demand_ties() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let near = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut().entity_mut(near).insert(OwnerSwarm(swarm));
    let far = common::spawn_operational_charger_at(&mut app, IVec2::new(1, 0), 0);
    app.world_mut().entity_mut(far).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

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
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let first = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut().entity_mut(first).insert(OwnerSwarm(swarm));
    let second = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
    app.world_mut().entity_mut(second).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

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

fn wait_for_assignment(app: &mut App, hauler: Entity) {
    for _ in 0..120 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            return;
        }
    }
    panic!("terminal request remained unresolved after 120 navigation ticks");
}

#[test]
fn starving_facility_beats_a_nearer_facility_whose_next_cycle_is_funded() {
    let mut app = common::sim_app_with_gather_haul();
    {
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.production.cost_per_bot = 5;
        tuning.production.input_capacity = 5;
        tuning.logistics.hauler_carry_capacity = 10;
    }
    let tuning = app.world().resource::<Tuning>().clone();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::ZERO, 100, 100);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    // Near facility: a full hopper funds its next cycle; a load would queue.
    let mut funded = ProductionFacility::with_tuning(&tuning);
    funded.input_amount = funded.input_capacity;
    funded.current_target = Some(NanobotType::Worker);
    funded.queued_space = 10;
    let funded = app
        .world_mut()
        .spawn((
            funded,
            OwnerSwarm(swarm),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ))
        .id();
    // Farther facility: empty while its swarm wants nanobots.
    let mut starving = ProductionFacility::with_tuning(&tuning);
    starving.demand_waiting = true;
    starving.queued_space = 10;
    let starving = app
        .world_mut()
        .spawn((
            starving,
            OwnerSwarm(swarm),
            Transform::from_xyz(-400.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 80.0));

    wait_for_assignment(&mut app, hauler);

    let sink_of = app.world().get::<HaulerAssignment>(hauler).unwrap().sink;
    assert_ne!(sink_of, funded, "a queued load cannot raise output sooner");
    assert_eq!(sink_of, starving);
}

/// A Sink holding `stock` with an inbound claim from a Source, and an empty
/// facility beside it whose swarm wants nanobots.
fn refilling_sink_with_hungry_facility(stock: u32) -> (App, Entity, Entity, Entity) {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), stock, 100);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let source = common::spawn_stockpile(&mut app, Vec2::new(-3_000.0, 0.0), 0, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    // Another Hauler is already bringing a load into the Sink.
    app.world_mut().spawn(LogisticsReservation::new(
        source,
        sink,
        top_down_2d_rts_prototype_nano_swarm::resources::ResourceKind::Minerals,
        10,
    ));
    let mut hungry = ProductionFacility::new();
    hungry.demand_waiting = true;
    let facility = app
        .world_mut()
        .spawn((
            hungry,
            OwnerSwarm(swarm),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ))
        .id();
    (app, swarm, sink, facility)
}

#[test]
fn haulers_wait_for_a_full_load_from_a_sink_being_refilled() {
    let (mut app, _, sink, _) = refilling_sink_with_hungry_facility(3);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-200.0, 100.0));
    for _ in 0..120 {
        app.update();
    }
    assert!(
        app.world()
            .get::<HaulerAssignment>(hauler)
            .is_none_or(|assignment| assignment.source != sink),
        "a 3-mineral sliver is not worth a round trip while the Sink refills"
    );

    // Once a full load is unclaimed, the facility is served.
    app.world_mut()
        .get_mut::<top_down_2d_rts_prototype_nano_swarm::resources::Stockpile>(sink)
        .unwrap()
        .amount = 30;
    wait_for_assignment(&mut app, hauler);
    assert_eq!(
        app.world().get::<HaulerAssignment>(hauler).unwrap().source,
        sink
    );
}

#[test]
fn a_charger_emergency_takes_a_sliver_from_a_sink_being_refilled() {
    let (mut app, swarm, sink, _) = refilling_sink_with_hungry_facility(3);
    let charger = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 0);
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
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-200.0, 100.0));

    wait_for_assignment(&mut app, hauler);

    let assignment = app.world().get::<HaulerAssignment>(hauler).unwrap();
    assert_eq!(assignment.source, sink);
    assert_eq!(assignment.sink, charger);
}

#[test]
fn a_refilling_sliver_does_not_pin_a_terminal_away_from_a_full_sink() {
    let (mut app, swarm, near, facility) = refilling_sink_with_hungry_facility(3);
    // Farther from the facility than the refilling Sink, but holding full loads.
    let far = common::spawn_sink_stockpile(&mut app, Vec2::new(1_000.0, 0.0), 100, 100);
    app.world_mut().entity_mut(far).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(600.0, 100.0));

    wait_for_assignment(&mut app, hauler);

    let assignment = app.world().get::<HaulerAssignment>(hauler).unwrap();
    assert_ne!(assignment.source, near);
    assert_eq!(assignment.source, far);
    assert_eq!(assignment.sink, facility);
}

#[test]
fn a_sliver_with_nothing_inbound_is_still_delivered() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 3, 100);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let mut hungry = ProductionFacility::new();
    hungry.demand_waiting = true;
    let facility = app
        .world_mut()
        .spawn((
            hungry,
            OwnerSwarm(swarm),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ))
        .id();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-200.0, 100.0));

    wait_for_assignment(&mut app, hauler);

    let assignment = app.world().get::<HaulerAssignment>(hauler).unwrap();
    assert_eq!(assignment.source, sink);
    assert_eq!(assignment.sink, facility);
}
