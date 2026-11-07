use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentGrid,
    nanobot::{
        AllocationClock, Charge, ChargerAssignment, DirectMovementComponent, HaulerAssignment,
        InteractionRegion, LogisticsReservation, OwnerSwarm, ProductionFacility,
        RegionalAllocationWake, RegionalLease, TerminalDemandAges,
    },
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
    navigation_runtime::NavigationBudget,
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
fn larger_proportional_terminal_deficit_beats_shorter_route() {
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let near_half_empty = common::spawn_operational_charger_at(&mut app, IVec2::ZERO, 30);
    app.world_mut()
        .entity_mut(near_half_empty)
        .insert(OwnerSwarm(swarm));
    let far_empty = common::spawn_operational_charger_at(&mut app, IVec2::new(1, 0), 0);
    app.world_mut()
        .entity_mut(far_empty)
        .insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-600.0, -200.0));

    wait_for_assignment(&mut app, hauler);

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
