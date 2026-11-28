//! Scripted near-runtime physical logistics custody flow.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::tuning::Tuning;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, Commitment, DirectMovementComponent, HaulerAssignment, HaulerLoading,
        InteractionRegion, LogisticsReservation, MaterialWorkProgress, OwnerSwarm,
        ProductionFacility, RegionalAllocationWake, SwarmId,
    },
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
    navigation_runtime::NavigationBudget,
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
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
    for _ in 0..500 {
        app.update();
        if cargo_amount(&app, hauler) > 0 {
            break;
        }
    }

    assert!(app.world().entity(hauler).contains::<HaulerLoading>());
    assert_eq!(
        cargo_amount(&app, hauler),
        Tuning::default().logistics.hauler_transfer_per_pulse
    );
    assert_eq!(
        stockpile_amount(&app, source),
        INITIAL_MINERALS - Tuning::default().logistics.hauler_transfer_per_pulse
    );
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(
        reservation.source_remaining,
        INITIAL_MINERALS - Tuning::default().logistics.hauler_transfer_per_pulse
    );
    assert_eq!(reservation.destination_remaining, INITIAL_MINERALS);
    assert_custody_and_ledger(&app, source, hauler, terminal);

    for _ in 0..2_000 {
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
    for _ in 0..Tuning::default().logistics.material_work_ticks {
        app.update();
    }
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
    for _ in 0..Tuning::default().logistics.material_work_ticks {
        app.update();
    }

    assert_eq!(
        facility_amount(&app, terminal),
        Tuning::default().logistics.hauler_transfer_per_pulse
    );
    assert_eq!(
        cargo_amount(&app, hauler),
        INITIAL_MINERALS - Tuning::default().logistics.hauler_transfer_per_pulse
    );
    let reservation = app
        .world()
        .entity(hauler)
        .get::<LogisticsReservation>()
        .unwrap();
    assert_eq!(reservation.source_remaining, 0);
    assert_eq!(
        reservation.destination_remaining,
        INITIAL_MINERALS - Tuning::default().logistics.hauler_transfer_per_pulse
    );
    assert_custody_and_ledger(&app, source, hauler, terminal);

    for _ in 0..2_000 {
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
fn a_single_physical_delivery_does_not_commit_another_facility() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind},
        nanobot::{
            Nanobot, NanobotType, PlannedKind, PlannedStructure, PlannedStructurePlugin,
            PopulationDemand, PopulationDemandPlugin, ProductionPlugin,
        },
    };

    let mut app = common::sim_app_with_gather_haul();
    app.add_plugins((
        PopulationDemandPlugin,
        ProductionPlugin,
        PlannedStructurePlugin,
    ));
    let cost = 5;
    {
        let mut tuning = app.world_mut().resource_mut::<Tuning>();
        tuning.production.cost_per_bot = cost;
        tuning.production.ticks_per_bot = 120;
        tuning.logistics.material_work_ticks = 24;
    }
    let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(440.0, 0.0));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-120.0, 0.0));
    let terminal = spawn_trip_terminal(&mut app, owner);
    let source = common::spawn_owned_source(&mut app, owner, Vec2::new(-200.0, 0.0), cost, 100);
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        cost,
    );
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(IVec2::ZERO, IntentKind::Build, SwarmId::PLAYER));
        for cell in [IVec2::X, IVec2::Y] {
            assert!(grid.paint(cell, IntentKind::Defend, SwarmId::PLAYER));
        }
    }

    let mut plans = app.world_mut().query::<&PlannedStructure>();
    let mut saw_production = false;
    let mut waited_for_minerals = false;
    for _ in 0..1_200 {
        app.update();
        let world = app.world();
        assert!(
            plans
                .iter(world)
                .all(|plan| plan.kind != PlannedKind::ProductionFacility),
            "one delivered cycle cannot justify another facility"
        );
        let facility = world.get::<ProductionFacility>(terminal).unwrap();
        saw_production |= facility.current_target == Some(NanobotType::Defender);
        if saw_production && facility.current_target.is_none() && facility.demand_waiting {
            waited_for_minerals = true;
            break;
        }
    }

    assert!(waited_for_minerals, "the facility runs once and starves");
    assert_eq!(stockpile_amount(&app, source), 0);
    assert_eq!(cargo_amount(&app, hauler), 0);
    assert_eq!(facility_amount(&app, terminal), 0);
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Defender),
        2
    );
    let world = app.world_mut();
    assert_eq!(
        world
            .query_filtered::<&NanobotType, With<Nanobot>>()
            .iter(world)
            .filter(|&&kind| kind == NanobotType::Defender)
            .count(),
        1
    );
}

#[test]
fn losing_either_endpoint_during_empty_pickup_allows_reassignment() {
    for lose_pickup in [true, false] {
        let (mut app, owner, mut source, mut terminal, hauler) = terminal_trip();
        for _ in 0..300 {
            app.update();
            if app.world().get::<HaulerLoading>(hauler).is_some() {
                break;
            }
        }
        assert!(app.world().get::<HaulerLoading>(hauler).is_some());
        assert_eq!(app.world().get::<Cargo>(hauler).unwrap().amount, 0);
        assert_custody_and_ledger(&app, source, hauler, terminal);

        let lost = if lose_pickup { source } else { terminal };
        assert!(app.world_mut().despawn(lost));
        app.update();
        assert!(app.world().get::<Cargo>(hauler).is_none());
        assert!(app.world().get::<MaterialWorkProgress>(hauler).is_none());
        assert!(app.world().get::<LogisticsReservation>(hauler).is_none());

        if lose_pickup {
            source = spawn_trip_pickup(&mut app, owner);
        } else {
            terminal = spawn_trip_terminal(&mut app, owner);
        }

        for _ in 0..500 {
            app.update();
            if cargo_amount(&app, hauler) > 0 {
                break;
            }
        }
        let assignment = app
            .world()
            .get::<HaulerAssignment>(hauler)
            .expect("the interrupted Hauler should acquire the replacement trip");
        assert_eq!((assignment.source, assignment.sink), (source, terminal));
        assert!(cargo_amount(&app, hauler) > 0);
        assert_custody_and_ledger(&app, source, hauler, terminal);
    }
}

#[test]
fn losing_pickup_stockpile_during_delivery_preserves_terminal_cargo() {
    for lose_during_unload in [false, true] {
        let (mut app, _, source, terminal, hauler) = terminal_trip();
        let delivered_before_loss = if lose_during_unload {
            Tuning::default().logistics.hauler_transfer_per_pulse
        } else {
            0
        };
        for _ in 0..2_000 {
            app.update();
            if cargo_amount(&app, hauler) == INITIAL_MINERALS - delivered_before_loss
                && facility_amount(&app, terminal) == delivered_before_loss
                && app.world().get::<HaulerLoading>(hauler).is_none()
            {
                break;
            }
        }
        assert!(app.world().get::<HaulerLoading>(hauler).is_none());
        assert_eq!(stockpile_amount(&app, source), 0);
        assert_eq!(facility_amount(&app, terminal), delivered_before_loss);
        assert_eq!(
            cargo_amount(&app, hauler),
            INITIAL_MINERALS - delivered_before_loss
        );
        assert!(app.world_mut().despawn(source));

        for _ in 0..2_000 {
            app.update();
            if facility_amount(&app, terminal) == INITIAL_MINERALS {
                break;
            }
        }
        assert_eq!(facility_amount(&app, terminal), INITIAL_MINERALS);
        assert!(app.world().get::<Cargo>(hauler).is_none());
        assert!(app.world().get::<HaulerAssignment>(hauler).is_none());
        assert!(app.world().get::<LogisticsReservation>(hauler).is_none());
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
            INITIAL_MINERALS,
        );
    }
}

fn terminal_trip() -> (App, Entity, Entity, Entity, Entity) {
    let mut app = common::sim_app_with_gather_haul();
    let owner = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = spawn_trip_pickup(&mut app, owner);
    let terminal = spawn_trip_terminal(&mut app, owner);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-120.0, 0.0));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        INITIAL_MINERALS,
    );
    (app, owner, source, terminal, hauler)
}

fn spawn_trip_pickup(app: &mut App, owner: Entity) -> Entity {
    let source = common::spawn_sink_stockpile(app, Vec2::new(-200.0, 0.0), INITIAL_MINERALS, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(owner));
    source
}

fn spawn_trip_terminal(app: &mut App, owner: Entity) -> Entity {
    app.world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(owner),
            Transform::from_xyz(200.0, 0.0, 0.0),
        ))
        .id()
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
        HaulerAssignment {
            source,
            sink,
            pickup_role: StockpileRole::Source,
        },
        reservation,
    ));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        12,
    );
    for _ in 0..2_000 {
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
        HaulerAssignment {
            source,
            sink,
            pickup_role: StockpileRole::Source,
        },
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
    for _ in 0..2_000 {
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
    for _ in 0..2_000 {
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
                pickup_role: StockpileRole::Source,
            },
            reservation,
        ));
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(swarm, ResourceKind::Minerals, 12);
        for _ in 0..2_000 {
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
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_stockpile(&mut app, Vec2::new(150.0, 0.0), 20, 100);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(400.0, 0.0), 0, 100);
    for endpoint in [source, sink] {
        app.world_mut()
            .entity_mut(endpoint)
            .insert(OwnerSwarm(swarm));
    }
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        20,
    );
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    for _ in 0..1_000 {
        app.update();
        if app
            .world()
            .get::<Cargo>(hauler)
            .is_some_and(|cargo| cargo.amount > 0)
        {
            break;
        }
    }
    let partial_cargo = app
        .world()
        .get::<Cargo>(hauler)
        .expect("hauler must physically load before its source is interrupted")
        .amount;
    assert!(partial_cargo < INITIAL_MINERALS);
    assert!(app.world().get::<HaulerLoading>(hauler).is_some());

    app.world_mut().get_mut::<Stockpile>(source).unwrap().amount = 0;
    for _ in 0..1_000 {
        app.update();
        if app.world().get::<HaulerLoading>(hauler).is_none() {
            break;
        }
    }
    assert!(
        app.world().get::<HaulerLoading>(hauler).is_none(),
        "an empty committed source ends the loading phase"
    );
    assert_eq!(
        app.world().get::<Cargo>(hauler).map(|cargo| cargo.amount),
        Some(partial_cargo),
        "interrupting the remaining supply preserves the already physical cargo"
    );

    for _ in 0..2_000 {
        app.update();
        if stockpile_amount(&app, sink) == partial_cargo {
            break;
        }
    }
    assert_eq!(stockpile_amount(&app, sink), partial_cargo);
    assert!(app.world().get::<Cargo>(hauler).is_none());
    assert!(app.world().get::<LogisticsReservation>(hauler).is_none());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        20,
        "the interruption moves no already-owned cargo out of the swarm ledger"
    );
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
            pickup_role: StockpileRole::Sink,
        },
        reservation,
    ));
    for _ in 0..2_000 {
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

const RESTART_COST: u32 = 5;
const RESTART_CYCLE_TICKS: u32 = 50;
const RESTART_TRANSFER_TICKS: u32 = 18;

struct RestartRefill {
    app: App,
    slow_hauler: Entity,
    fast_hauler: Entity,
    light_pickup: Entity,
    full_pickup: Entity,
    facility: Entity,
}

impl RestartRefill {
    fn new(hopper: u32, inbound: u32, light_stock: u32, slow_position: Vec2) -> Self {
        use top_down_2d_rts_prototype_nano_swarm::nanobot::{
            PopulationDemandPlugin, ProductionPlugin,
        };

        let mut app = common::sim_app_with_gather_haul();
        app.add_plugins((PopulationDemandPlugin, ProductionPlugin));
        {
            let mut tuning = app.world_mut().resource_mut::<Tuning>();
            tuning.production.cost_per_bot = RESTART_COST;
            tuning.production.input_capacity = 2 * RESTART_COST;
            tuning.production.ticks_per_bot = RESTART_CYCLE_TICKS;
            tuning.logistics.hauler_carry_capacity = 2 * RESTART_COST;
            tuning.logistics.hauler_transfer_per_pulse = 1;
            tuning.logistics.material_work_ticks = RESTART_TRANSFER_TICKS;
        }
        let facility_pos = Vec2::new(36.0, 36.0);
        let owner = common::spawn_swarm_at(&mut app, facility_pos);
        let facility = common::spawn_facility_with_hopper(&mut app, owner, facility_pos, hopper);
        let slow_hauler = common::spawn_hauler_at(&mut app, slow_position);
        let fast_hauler = common::spawn_hauler_at(&mut app, Vec2::new(36.0, -120.0));
        app.update();
        assert!(
            !app.world()
                .get::<ProductionFacility>(facility)
                .unwrap()
                .demand_waiting
        );
        let light_pickup =
            common::spawn_owned_source(&mut app, owner, Vec2::new(-144.0, 36.0), light_stock, 100);
        let full_pickup = common::spawn_owned_source(
            &mut app,
            owner,
            Vec2::new(216.0, 36.0),
            2 * RESTART_COST,
            100,
        );
        let mut claim =
            LogisticsReservation::new(full_pickup, facility, ResourceKind::Minerals, inbound);
        claim.source_remaining = 0;
        app.world_mut().entity_mut(slow_hauler).insert((
            HaulerAssignment {
                source: full_pickup,
                sink: facility,
                pickup_role: StockpileRole::Source,
            },
            claim,
            Cargo {
                kind: ResourceKind::Minerals,
                amount: inbound,
            },
        ));
        app.world_mut().resource_mut::<ResourceLedger>().add_for(
            SwarmId::PLAYER,
            ResourceKind::Minerals,
            hopper + inbound + light_stock + 2 * RESTART_COST,
        );
        Self {
            app,
            slow_hauler,
            fast_hauler,
            light_pickup,
            full_pickup,
            facility,
        }
    }

    fn return_defender_demand(&mut self) {
        use top_down_2d_rts_prototype_nano_swarm::intent::{IntentGrid, IntentKind};

        let mut grid = self.app.world_mut().resource_mut::<IntentGrid>();
        for y in 0..3 {
            for x in 0..4 {
                assert!(grid.paint(IVec2::new(x, y), IntentKind::Defend, SwarmId::PLAYER));
            }
        }
    }

    fn cycle_starts(&mut self, ticks: u32) -> Vec<u32> {
        use top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType;

        let mut starts = Vec::new();
        for tick in 1..=ticks {
            self.app.update();
            let production = self
                .app
                .world()
                .get::<ProductionFacility>(self.facility)
                .unwrap();
            if production.current_target == Some(NanobotType::Defender) && production.progress == 1
            {
                starts.push(tick);
            }
        }
        starts
    }

    fn picked_source(&self) -> Option<Entity> {
        self.app
            .world()
            .get::<HaulerAssignment>(self.fast_hauler)
            .map(|assignment| assignment.source)
    }

    fn warm_pickup_connectivity(&self, hauler: Entity, source: Entity) -> ConnectivityStatus {
        use top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid;

        let world = self.app.world();
        let navigation = world.resource::<Navigation>();
        let grid = world.resource::<IntentGrid>();
        let budget = world.resource::<NavigationBudget>().0;
        let start = world
            .get::<Transform>(hauler)
            .unwrap()
            .translation
            .truncate();
        let connect = |origin, target| {
            let region = InteractionRegion::structure(world.get::<Transform>(target).unwrap());
            for _ in 0..60 {
                let status = navigation.query_connectivity(origin, RouteGoal::Interaction(region));
                if status != ConnectivityStatus::Pending {
                    return status;
                }
                navigation.advance(grid, budget);
            }
            panic!("pickup connectivity should resolve")
        };
        match connect(start, source) {
            ConnectivityStatus::Connected { endpoint } => connect(endpoint, self.facility),
            status => status,
        }
    }

    fn acquire(&mut self) {
        self.app
            .world_mut()
            .resource_mut::<RegionalAllocationWake>()
            .request_current_pass();
        self.app.update();
    }
}

#[test]
fn faster_refill_restarts_an_empty_or_stocked_facility_before_the_slow_claim() {
    for initial_cycles in [0, 2] {
        let mut fixture = RestartRefill::new(
            initial_cycles * RESTART_COST,
            RESTART_COST,
            RESTART_COST,
            Vec2::new(1800.0, 200.0),
        );
        fixture.return_defender_demand();

        let mut starts = fixture.cycle_starts(30);
        assert_eq!(
            fixture.picked_source(),
            Some(fixture.light_pickup),
            "the pickup with five minerals should fund the restart before the full pickup",
        );
        assert_eq!(
            cargo_amount(&fixture.app, fixture.slow_hauler),
            RESTART_COST
        );
        starts.extend(fixture.cycle_starts(240).into_iter().map(|tick| tick + 30));
        assert_eq!(starts.len() as u32, initial_cycles + 1);
        assert_eq!(
            cargo_amount(&fixture.app, fixture.slow_hauler),
            RESTART_COST
        );
    }
}

fn add_restart_rock_wall(app: &mut App, half_height: f32) {
    common::spawn_rock_rectangle(app, Vec2::new(-54.0, 36.0), Vec2::new(18.0, half_height));
}

#[test]
fn a_detour_changes_which_pickup_funds_the_restart_first() {
    let mut fixture = RestartRefill::new(0, RESTART_COST, RESTART_COST, Vec2::new(1800.0, 200.0));
    // The light pickup has a shorter straight-line round trip, but a rock
    // forces a long detour to reach it and carry its minerals back.
    add_restart_rock_wall(&mut fixture.app, 540.0);
    fixture
        .app
        .world_mut()
        .get_mut::<Transform>(fixture.full_pickup)
        .unwrap()
        .translation
        .x = 345.0;
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.fast_hauler)
        .insert(Commitment::Working);
    assert!(fixture.cycle_starts(1).is_empty());
    for source in [fixture.light_pickup, fixture.full_pickup] {
        assert!(matches!(
            fixture.warm_pickup_connectivity(fixture.fast_hauler, source),
            ConnectivityStatus::Connected { .. }
        ));
    }
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.fast_hauler)
        .insert(Commitment::Idle);
    fixture.return_defender_demand();

    assert!(fixture.cycle_starts(40).is_empty());
    assert_eq!(fixture.picked_source(), Some(fixture.full_pickup));
    assert_eq!(
        fixture
            .app
            .world()
            .get::<LogisticsReservation>(fixture.fast_hauler)
            .unwrap()
            .amount,
        RESTART_COST,
    );
    assert_eq!(fixture.cycle_starts(285).len(), 1);
    assert_eq!(
        stockpile_amount(&fixture.app, fixture.light_pickup),
        RESTART_COST
    );
    assert_eq!(
        cargo_amount(&fixture.app, fixture.slow_hauler),
        RESTART_COST
    );
}

#[test]
fn a_restart_refill_dispatches_with_a_sealed_pickup_or_a_proven_advancement() {
    for (wall_height, inbound, hopper, light_reachable, expected_amount) in [
        (540.0, Vec2::new(1800.0, 200.0), 0, true, RESTART_COST),
        (2100.0, Vec2::new(36.0, 125.0), 4, false, 2 * RESTART_COST),
    ] {
        let mut fixture = RestartRefill::new(hopper, RESTART_COST, RESTART_COST, inbound);
        add_restart_rock_wall(&mut fixture.app, wall_height);
        fixture
            .app
            .world_mut()
            .entity_mut(fixture.fast_hauler)
            .insert(Commitment::Working);
        fixture.app.update();
        assert!(matches!(
            fixture.warm_pickup_connectivity(fixture.fast_hauler, fixture.full_pickup),
            ConnectivityStatus::Connected { .. }
        ));
        assert_eq!(
            matches!(
                fixture.warm_pickup_connectivity(fixture.fast_hauler, fixture.light_pickup),
                ConnectivityStatus::Connected { .. }
            ),
            light_reachable
        );
        fixture
            .app
            .world_mut()
            .entity_mut(fixture.fast_hauler)
            .insert(Commitment::Idle);
        fixture.app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
        fixture.return_defender_demand();

        fixture.acquire();
        assert_eq!(
            fixture.picked_source(),
            Some(fixture.full_pickup),
            "light pickup reachable: {light_reachable}",
        );
        assert_eq!(
            fixture
                .app
                .world()
                .get::<LogisticsReservation>(fixture.fast_hauler)
                .unwrap()
                .amount,
            expected_amount,
        );
    }
}

#[test]
fn a_pending_inbound_refill_waits_even_at_an_aged_terminal_then_uses_the_usual_load() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::TerminalDemandAges;

    let mut fixture = RestartRefill::new(3, 4, 1, Vec2::new(1800.0, 200.0));
    add_restart_rock_wall(&mut fixture.app, 840.0);
    fixture
        .app
        .world_mut()
        .get_mut::<Transform>(fixture.light_pickup)
        .unwrap()
        .translation
        .x = -220.0;
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.fast_hauler)
        .insert(Commitment::Working);
    assert!(fixture.cycle_starts(31).is_empty());
    assert!(
        fixture
            .app
            .world()
            .resource::<TerminalDemandAges>()
            .waiting_ticks(fixture.facility)
            > 4
    );
    assert!(matches!(
        fixture.warm_pickup_connectivity(fixture.fast_hauler, fixture.full_pickup),
        ConnectivityStatus::Connected { .. }
    ));
    let world = fixture.app.world_mut();
    let pickup_position = Vec2::new(-152.0, 36.0);
    let pickup =
        InteractionRegion::structure(world.get::<Transform>(fixture.light_pickup).unwrap());
    assert!(pickup.contains(pickup_position));
    let mut inbound = LogisticsReservation::new(
        fixture.light_pickup,
        fixture.facility,
        ResourceKind::Minerals,
        RESTART_COST,
    );
    inbound.source_remaining = 1;
    world
        .entity_mut(fixture.slow_hauler)
        .remove::<DirectMovementComponent>()
        .insert((
            Transform::from_translation(pickup_position.extend(0.0)),
            HaulerAssignment {
                source: fixture.light_pickup,
                sink: fixture.facility,
                pickup_role: StockpileRole::Source,
            },
            inbound,
            HaulerLoading,
            MaterialWorkProgress { ticks_elapsed: 0 },
        ));
    world
        .entity_mut(fixture.fast_hauler)
        .insert(Commitment::Idle);
    world.resource_mut::<NavigationBudget>().0 = 0;
    fixture.return_defender_demand();

    for _ in 0..4 {
        fixture.acquire();
        assert_eq!(fixture.picked_source(), None);
    }
    fixture.acquire();
    assert_eq!(fixture.picked_source(), Some(fixture.full_pickup));
    let world = fixture.app.world();
    let fallback = world
        .get::<LogisticsReservation>(fixture.fast_hauler)
        .unwrap();
    let facility = world.get::<ProductionFacility>(fixture.facility).unwrap();
    let inbound = world
        .get::<LogisticsReservation>(fixture.slow_hauler)
        .unwrap();
    assert_eq!(inbound.destination_remaining, RESTART_COST);
    assert_eq!(fallback.amount, 2 * RESTART_COST);
    assert!(fallback.amount > RESTART_COST - facility.input_amount);
}

#[test]
fn an_untimed_dispatch_defers_timing_a_second_refill_in_the_same_pass() {
    let mut fixture = RestartRefill::new(3, 0, RESTART_COST, Vec2::new(-152.0, 36.0));
    add_restart_rock_wall(&mut fixture.app, 840.0);
    fixture
        .app
        .world_mut()
        .get_mut::<Transform>(fixture.light_pickup)
        .unwrap()
        .translation
        .x = -220.0;
    let world = fixture.app.world_mut();
    world
        .resource_mut::<Tuning>()
        .autonomy
        .terminal_fairness_promotion_ticks = 1;
    world
        .entity_mut(fixture.slow_hauler)
        .remove::<(HaulerAssignment, LogisticsReservation, Cargo)>();
    let mut haulers = [fixture.slow_hauler, fixture.fast_hauler];
    haulers.sort_by_key(|hauler| hauler.to_bits());
    let [first, second] = haulers;
    world
        .entity_mut(first)
        .insert((Transform::from_xyz(-152.0, 36.0, 0.0), Commitment::Working));
    world
        .entity_mut(second)
        .insert((Transform::from_xyz(36.0, -120.0, 0.0), Commitment::Working));
    assert!(fixture.cycle_starts(31).is_empty());
    for (hauler, source) in [(first, fixture.light_pickup), (second, fixture.full_pickup)] {
        assert!(matches!(
            fixture.warm_pickup_connectivity(hauler, source),
            ConnectivityStatus::Connected { .. }
        ));
    }
    let world = fixture.app.world_mut();
    for hauler in haulers {
        world.entity_mut(hauler).insert(Commitment::Idle);
    }
    world.resource_mut::<NavigationBudget>().0 = 0;
    fixture.return_defender_demand();
    fixture.acquire();

    let world = fixture.app.world();
    let first_claim = world.get::<LogisticsReservation>(first).unwrap();
    assert_eq!(
        (first_claim.source, first_claim.destination),
        (fixture.light_pickup, fixture.facility)
    );
    assert_eq!(first_claim.amount, RESTART_COST);
    assert!(world.get::<LogisticsReservation>(second).is_none());
}

#[test]
fn an_inbound_refill_across_a_detour_does_not_hide_a_timely_pickup() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::InteractionRegion;

    for (wall, expected_pickup) in [(false, 2 * RESTART_COST), (true, RESTART_COST)] {
        let pickup_position = Vec2::new(-152.0, 36.0);
        let mut fixture =
            RestartRefill::new(2 * RESTART_COST, RESTART_COST - 1, 1, pickup_position);
        fixture
            .app
            .world_mut()
            .get_mut::<Transform>(fixture.light_pickup)
            .unwrap()
            .translation
            .x = -220.0;
        if wall {
            add_restart_rock_wall(&mut fixture.app, 840.0);
        }
        fixture
            .app
            .world_mut()
            .resource_mut::<Tuning>()
            .production
            .ticks_per_bot = 125;

        let world = fixture.app.world_mut();
        let pickup =
            InteractionRegion::structure(world.get::<Transform>(fixture.light_pickup).unwrap());
        assert!(pickup.contains(pickup_position));
        let mut claim = LogisticsReservation::new(
            fixture.light_pickup,
            fixture.facility,
            ResourceKind::Minerals,
            RESTART_COST,
        );
        claim.source_remaining = 1;
        world.entity_mut(fixture.slow_hauler).insert((
            HaulerAssignment {
                source: fixture.light_pickup,
                sink: fixture.facility,
                pickup_role: StockpileRole::Source,
            },
            claim,
            HaulerLoading,
            MaterialWorkProgress { ticks_elapsed: 0 },
        ));
        fixture.return_defender_demand();

        assert_eq!(fixture.cycle_starts(18), vec![1]);
        assert_eq!(fixture.picked_source(), Some(fixture.full_pickup));
        let claim = fixture
            .app
            .world()
            .get::<LogisticsReservation>(fixture.fast_hauler)
            .unwrap();
        assert_eq!(claim.amount, expected_pickup, "rock wall present: {wall}");
        assert_eq!(fixture.cycle_starts(247).len(), 2);
        if wall {
            assert_eq!(
                cargo_amount(&fixture.app, fixture.slow_hauler),
                RESTART_COST
            );
        }
    }
}

#[test]
fn charger_emergency_precedes_a_restart_refill() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Charge, ChargerAssignment};

    let mut fixture = RestartRefill::new(0, RESTART_COST, RESTART_COST, Vec2::new(1800.0, 200.0));
    add_restart_rock_wall(&mut fixture.app, 540.0);
    let owner = *fixture
        .app
        .world()
        .get::<OwnerSwarm>(fixture.facility)
        .unwrap();
    let charger = common::spawn_operational_charger_at(&mut fixture.app, IVec2::new(1, -1), 0);
    fixture.app.world_mut().entity_mut(charger).insert(owner);
    let defender = common::spawn_defender_at(&mut fixture.app, Vec2::new(700.0, -250.0));
    fixture.app.world_mut().entity_mut(defender).insert((
        Charge {
            current: 0.0,
            max: 1.0,
        },
        ChargerAssignment { charger },
    ));
    fixture.return_defender_demand();

    assert!(fixture.cycle_starts(40).is_empty());
    let production = fixture
        .app
        .world()
        .get::<ProductionFacility>(fixture.facility)
        .unwrap();
    assert!(production.demand_waiting && production.input_amount == 0);
    assert_eq!(
        fixture
            .app
            .world()
            .get::<HaulerAssignment>(fixture.fast_hauler)
            .map(|assignment| assignment.sink),
        Some(charger),
    );
}

#[test]
fn growing_demand_keeps_the_restart_boundary_during_the_funded_burst() {
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind},
        nanobot::{Commitment, NanobotType, PopulationDemand},
    };

    let mut fixture = RestartRefill::new(
        2 * RESTART_COST,
        RESTART_COST,
        RESTART_COST,
        Vec2::new(1800.0, 200.0),
    );
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.fast_hauler)
        .insert(Commitment::Working);
    for x in 0..4 {
        assert!(fixture.app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::new(x, 0),
            IntentKind::Defend,
            SwarmId::PLAYER,
        ));
    }
    assert_eq!(fixture.cycle_starts(12).len(), 1);
    assert_eq!(
        fixture
            .app
            .world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Defender),
        2
    );
    assert_eq!(
        facility_amount(&fixture.app, fixture.facility),
        RESTART_COST
    );
    assert!(fixture.picked_source().is_none());

    for x in 0..2 {
        assert!(fixture.app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::new(x, 1),
            IntentKind::Defend,
            SwarmId::PLAYER,
        ));
    }
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.fast_hauler)
        .insert(Commitment::Idle);

    assert!(fixture.cycle_starts(28).is_empty());
    assert_eq!(fixture.picked_source(), Some(fixture.light_pickup));
    assert_eq!(fixture.cycle_starts(230).len(), 2);
    assert_eq!(
        cargo_amount(&fixture.app, fixture.slow_hauler),
        RESTART_COST
    );
}

#[test]
fn the_restart_can_use_minerals_from_two_individually_insufficient_unloads() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::InteractionRegion;

    let mut fixture = RestartRefill::new(0, 2, 3, Vec2::new(36.0, 150.0));
    let world = fixture.app.world_mut();
    let position = InteractionRegion::structure(world.get::<Transform>(fixture.facility).unwrap())
        .approach(Vec2::new(36.0, 150.0));
    world.entity_mut(fixture.slow_hauler).insert((
        Transform::from_translation(position.extend(0.0)),
        MaterialWorkProgress {
            ticks_elapsed: RESTART_TRANSFER_TICKS - 1,
        },
    ));
    fixture.return_defender_demand();
    assert!(fixture.cycle_starts(RESTART_TRANSFER_TICKS).is_empty());
    assert_eq!(facility_amount(&fixture.app, fixture.facility), 1);
    assert_eq!(cargo_amount(&fixture.app, fixture.slow_hauler), 1);
    assert_eq!(fixture.picked_source(), Some(fixture.light_pickup));
    assert!(fixture.cycle_starts(30 - RESTART_TRANSFER_TICKS).is_empty());
    assert_eq!(facility_amount(&fixture.app, fixture.facility), 2);
    assert_eq!(
        stockpile_amount(&fixture.app, fixture.full_pickup),
        2 * RESTART_COST
    );
    assert!((1..=155).any(|_| !fixture.cycle_starts(1).is_empty()));
    assert_eq!(facility_amount(&fixture.app, fixture.facility), 0);
}

#[test]
fn a_short_pickup_restarts_production_earlier_than_the_usual_load() {
    let starting_scene = || RestartRefill::new(3, 5, 0, Vec2::new(1800.0, 200.0));
    let mut shorter = starting_scene();
    let mut usual = starting_scene();
    for fixture in [&mut shorter, &mut usual] {
        fixture.return_defender_demand();
        assert!(fixture.cycle_starts(20).is_empty());
        assert_eq!(fixture.picked_source(), Some(fixture.full_pickup));
        assert_eq!(cargo_amount(&fixture.app, fixture.fast_hauler), 0);
    }
    let short_claim = shorter
        .app
        .world()
        .get::<LogisticsReservation>(shorter.fast_hauler)
        .unwrap();
    assert_eq!(
        (
            short_claim.amount,
            short_claim.source_remaining,
            short_claim.destination_remaining
        ),
        (2, 2, 2)
    );
    // Use the usual full load on the same trip before either pickup begins.
    usual
        .app
        .world_mut()
        .entity_mut(usual.fast_hauler)
        .insert(LogisticsReservation::new(
            usual.full_pickup,
            usual.facility,
            ResourceKind::Minerals,
            2 * RESTART_COST,
        ));

    assert_eq!(shorter.cycle_starts(130).len(), 1);
    assert!(usual.cycle_starts(130).is_empty());
    assert_eq!(facility_amount(&shorter.app, shorter.facility), 0);
    assert_eq!(cargo_amount(&shorter.app, shorter.fast_hauler), 0);
    assert_eq!(stockpile_amount(&shorter.app, shorter.full_pickup), 8);
    assert_eq!(cargo_amount(&shorter.app, shorter.slow_hauler), 5);
    assert_eq!(usual.cycle_starts(160).len(), 1);
}

#[test]
fn a_short_pickup_funds_the_restart_with_part_of_a_larger_inbound_load() {
    let mut fixture = RestartRefill::new(0, 7, 0, Vec2::new(590.0, 36.0));
    fixture.return_defender_demand();
    assert!(fixture.cycle_starts(20).is_empty());
    let claim = fixture
        .app
        .world()
        .get::<LogisticsReservation>(fixture.fast_hauler)
        .unwrap();
    let amount = claim.amount;
    assert!((1..RESTART_COST).contains(&amount), "picked up {amount}");
    assert_eq!(claim.source_remaining, amount);
    assert_eq!(claim.destination_remaining, amount);

    let restart_tick = (21..=240).find(|_| !fixture.cycle_starts(1).is_empty());
    assert!(
        restart_tick.is_some(),
        "the {amount}-mineral refill did not fund the first cycle by tick 240"
    );
    let inbound_delivered = 7 - cargo_amount(&fixture.app, fixture.slow_hauler);
    assert!(
        (1..RESTART_COST).contains(&inbound_delivered),
        "at tick {restart_tick:?}, the inbound trip delivered {inbound_delivered} and the {amount}-mineral refill carried {}",
        cargo_amount(&fixture.app, fixture.fast_hauler),
    );
    assert_eq!(
        inbound_delivered + amount - cargo_amount(&fixture.app, fixture.fast_hauler),
        RESTART_COST + facility_amount(&fixture.app, fixture.facility)
    );
}

#[test]
fn equal_restart_funding_times_keep_the_usual_pickup() {
    let mut fixture = RestartRefill::new(3, 5, 0, Vec2::new(1800.0, 200.0));
    fixture
        .app
        .world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .hauler_transfer_per_pulse = 10;
    fixture.return_defender_demand();
    assert!(fixture.cycle_starts(20).is_empty());
    assert_eq!(fixture.picked_source(), Some(fixture.full_pickup));
    let claim = fixture
        .app
        .world()
        .get::<LogisticsReservation>(fixture.fast_hauler)
        .unwrap();
    assert_eq!(claim.amount, 2 * RESTART_COST);
    assert_eq!(fixture.cycle_starts(120).len(), 1);
    assert_eq!(stockpile_amount(&fixture.app, fixture.full_pickup), 0);
    assert_eq!(facility_amount(&fixture.app, fixture.facility), 8);
}

#[test]
fn a_full_restart_window_preserves_the_slow_claim_until_physical_delivery() {
    let load = 2 * RESTART_COST;
    let mut fixture = RestartRefill::new(0, load, RESTART_COST, Vec2::new(1500.0, 180.0));
    let second_hauler = common::spawn_hauler_at(&mut fixture.app, Vec2::new(1600.0, -120.0));
    let mut claim = LogisticsReservation::new(
        fixture.full_pickup,
        fixture.facility,
        ResourceKind::Minerals,
        load,
    );
    claim.source_remaining = 0;
    fixture.app.world_mut().entity_mut(second_hauler).insert((
        HaulerAssignment {
            source: fixture.full_pickup,
            sink: fixture.facility,
            pickup_role: StockpileRole::Source,
        },
        claim,
        Cargo {
            kind: ResourceKind::Minerals,
            amount: load,
        },
    ));
    fixture
        .app
        .world_mut()
        .resource_mut::<ResourceLedger>()
        .add_for(SwarmId::PLAYER, ResourceKind::Minerals, load);
    {
        let mut facility = fixture
            .app
            .world_mut()
            .get_mut::<ProductionFacility>(fixture.facility)
            .unwrap();
        facility.queued_space = load;
        facility.measured_lead_ticks = Some(1.0);
    }
    fixture.return_defender_demand();

    assert!(fixture.cycle_starts(250).is_empty());
    assert_eq!(fixture.picked_source(), None);
    assert_eq!(facility_amount(&fixture.app, fixture.facility), 0);
    for hauler in [fixture.slow_hauler, second_hauler] {
        assert_eq!(
            fixture.app.world().get::<LogisticsReservation>(hauler),
            Some(&claim)
        );
        assert_eq!(cargo_amount(&fixture.app, hauler), load);
    }
    assert!(!fixture.cycle_starts(180).is_empty());
}
