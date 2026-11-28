//! Scripted near-runtime physical logistics custody flow.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::tuning::Tuning;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        Cargo, DirectMovementComponent, HaulerAssignment, HaulerLoading, LogisticsReservation,
        MaterialWorkProgress, OwnerSwarm, ProductionFacility, SwarmId,
    },
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
        let tuning = app.world().resource::<Tuning>().clone();
        let owner = common::spawn_swarm_at(&mut app, Vec2::new(36.0, 36.0));
        let mut production = ProductionFacility::with_tuning(&tuning);
        production.input_amount = hopper;
        let facility = app
            .world_mut()
            .spawn((
                production,
                OwnerSwarm(owner),
                Transform::from_xyz(36.0, 36.0, 0.0),
            ))
            .id();
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
    assert_eq!(fixture.cycle_starts(155).len(), 1);
    assert_eq!(facility_amount(&fixture.app, fixture.facility), 0);
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
