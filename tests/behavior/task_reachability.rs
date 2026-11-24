use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{NanobotType, PopulationDemand, SwarmId},
    navigation_runtime::NavigationBudget,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn unreachable_work_stops_population_demand_and_reopening_restores_it() {
    let mut app = common::sim_app_with_population_demand();
    app.insert_resource(IntentGrid::new(2, 2));
    common::spawn_swarm_at(&mut app, Vec2::new(-200.0, 100.0));
    common::spawn_worker_at(&mut app, Vec2::new(-200.0, 100.0));
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(200.0, 100.0),
            amount: 100,
            capacity: 100,
            radius: 32.0,
        },
    );
    let wall = common::spawn_structure_at(&mut app, Vec2::ZERO);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        1,
        "unknown access must preserve demand"
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32_768;
    for _ in 0..40 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        0,
        "a wall across the entire map proves work inaccessible"
    );
    app.world_mut().despawn(wall);
    for _ in 0..40 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        1,
        "removing the wall reopens the work"
    );
}

#[test]
fn reachable_haul_is_assigned_while_another_destination_search_is_pending() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{HaulerAssignment, InteractionRegion, OwnerSwarm, ProductionFacility},
        navigation::{ConnectivityStatus, Navigation, RouteGoal},
    };
    let mut app = common::sim_app_with_gather_haul();
    app.insert_resource(IntentGrid::new(2, 2));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let ready = common::spawn_facility_at(&mut app, swarm, Vec2::new(100.0, 0.0));
    let unknown = common::spawn_facility_at(&mut app, swarm, Vec2::new(300.0, 200.0));
    for facility in [ready, unknown] {
        app.world_mut()
            .get_mut::<ProductionFacility>(facility)
            .unwrap()
            .input_amount = 0;
    }
    app.update();
    let origin = Vec2::new(-350.0, 0.0);
    let source_region = InteractionRegion::structure(app.world().get::<Transform>(source).unwrap());
    let ready_region = InteractionRegion::structure(app.world().get::<Transform>(ready).unwrap());
    let navigation = app.world().resource::<Navigation>();
    let grid = app.world().resource::<IntentGrid>();
    let mut pickup = None;
    for _ in 0..40 {
        if let ConnectivityStatus::Connected { endpoint } =
            navigation.query_connectivity(origin, RouteGoal::Interaction(source_region))
        {
            pickup = Some(endpoint);
            break;
        }
        navigation.advance(grid, 32_768);
    }
    let pickup = pickup.expect("the clear pickup approach resolves within forty navigation ticks");
    for _ in 0..40 {
        if matches!(
            navigation.query_connectivity(pickup, RouteGoal::Interaction(ready_region)),
            ConnectivityStatus::Connected { .. }
        ) {
            break;
        }
        navigation.advance(grid, 32_768);
    }
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    let hauler = common::spawn_hauler_at(&mut app, origin);
    for _ in 0..60 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<HaulerAssignment>(hauler)
            .map(|assignment| assignment.sink),
        Some(ready),
        "a delayed competing search must not suppress already reachable work"
    );
}

#[test]
fn blocked_empty_hauler_releases_pickup_reservation() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        HaulerAssignment, LogisticsReservation, OwnerSwarm, ProductionFacility,
    };
    let mut app = common::sim_app_with_gather_haul();
    app.insert_resource(IntentGrid::new(2, 2));
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(-300.0, 0.0));
    let source = common::spawn_sink_stockpile(&mut app, Vec2::new(150.0, 0.0), 100, 100);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let terminal = common::spawn_facility_at(&mut app, swarm, Vec2::new(300.0, 150.0));
    app.world_mut()
        .get_mut::<ProductionFacility>(terminal)
        .unwrap()
        .input_amount = 0;
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-300.0, 0.0));
    for _ in 0..40 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<HaulerAssignment>(hauler)
            .map(|assignment| assignment.source),
        Some(source)
    );
    let wall = common::spawn_structure_at(&mut app, Vec2::ZERO);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    for _ in 0..60 {
        app.update();
    }
    assert!(
        app.world().get::<HaulerAssignment>(hauler).is_none(),
        "proven failed pickup cannot retain the empty hauler"
    );
    assert!(
        app.world().get::<LogisticsReservation>(hauler).is_none(),
        "the abandoned pickup releases its reserved minerals and capacity"
    );
    assert_eq!(
        app.world()
            .get::<top_down_2d_rts_prototype_nano_swarm::resources::Stockpile>(source)
            .unwrap()
            .amount,
        100
    );
}

#[test]
fn gather_requires_contact_ground_in_an_owned_painted_tile() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{GatherAssignment, OwnerSwarm, world_to_cell},
        reachability::{IntentWriteAction, SwarmReachabilityMap, apply_reachable_intent},
        resources::{ResourceDeposit, StockpileRole},
        terrain::RockFormation,
        tuning::Tuning,
    };

    let mut app = common::sim_app_with_gather();
    let home = Vec2::new(96.0, 256.0);
    let swarm = common::spawn_swarm_at(&mut app, home);
    let worker = common::spawn_worker_at(&mut app, home);
    let source = common::spawn_stockpile(&mut app, Vec2::new(160.0, 155.0), 0, 10);
    app.world_mut()
        .entity_mut(source)
        .insert((OwnerSwarm(swarm), StockpileRole::Source));
    let mut terrain = Vec::new();
    for (center, half) in [
        (Vec2::new(256.0, 256.0), Vec2::new(20.0, 70.0)),
        (Vec2::new(450.0, 256.0), Vec2::new(30.0, 135.0)),
    ] {
        let rock = RockFormation::Rectangle { half };
        let transform = Transform::from_translation(center.extend(0.0));
        app.world_mut().spawn((rock, transform));
        terrain.push(rock.obstacle(&transform));
    }
    let reachable = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(144.0, 370.0),
            amount: 1,
            capacity: 1,
            radius: 32.0,
        },
    );
    let around_the_wall = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(505.0, 256.0),
            amount: 3,
            capacity: 3,
            radius: 22.0,
        },
    );
    let map = SwarmReachabilityMap::new(
        app.world().resource::<IntentGrid>(),
        terrain,
        [(SwarmId::PLAYER, home)],
    )
    .unwrap();
    app.insert_resource(map.clone());
    app.world_mut()
        .resource_mut::<Tuning>()
        .logistics
        .material_work_ticks = 2;
    let paint = |app: &mut App, cell| {
        assert!(
            apply_reachable_intent(
                &mut app.world_mut().resource_mut::<IntentGrid>(),
                &map,
                SwarmId::PLAYER,
                cell,
                IntentKind::Gather,
                IntentWriteAction::Paint
            )
            .unwrap()
        );
    };
    paint(&mut app, IVec2::ZERO);

    for _ in 0..550 {
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<ResourceDeposit>(reachable)
            .unwrap()
            .amount,
        0
    );
    assert_eq!(
        app.world()
            .get::<ResourceDeposit>(around_the_wall)
            .unwrap()
            .amount,
        3,
        "an accessible but unpainted exterior cannot perform the work"
    );
    assert!(app.world().get::<GatherAssignment>(worker).is_none());

    paint(&mut app, IVec2::X);
    for _ in 0..550 {
        app.update();
        if app
            .world()
            .get::<ResourceDeposit>(around_the_wall)
            .unwrap()
            .amount
            == 2
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<ResourceDeposit>(around_the_wall)
            .unwrap()
            .amount,
        2
    );
    assert_eq!(
        app.world().get::<GatherAssignment>(worker).unwrap().cell,
        IVec2::X,
        "the Worker uses the paint on the usable side of the wall"
    );
    let position = app
        .world()
        .get::<Transform>(worker)
        .unwrap()
        .translation
        .truncate();
    assert_eq!(world_to_cell(position), IVec2::X);
}

#[test]
fn worker_extracts_through_narrow_contact_ground_at_tile_edges() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            DirectMovementComponent, GatherAssignment, InteractionRegion, OwnerSwarm, world_to_cell,
        },
        reachability::{IntentWriteAction, SwarmReachabilityMap, apply_reachable_intent},
        resources::{ResourceDeposit, StockpileRole},
        terrain::RockFormation,
        tuning::Tuning,
    };

    for (home, deposit_position, radius, source_position, rocks) in [
        (
            Vec2::new(-1600.0, -1550.0),
            Vec2::new(331.0, 544.0),
            600.0,
            Vec2::new(-160.0, -300.0),
            &[][..],
        ),
        (
            Vec2::new(1600.0, 1600.0),
            Vec2::new(63.69218, 63.69218),
            600.0,
            Vec2::new(800.0, 620.0),
            &[][..],
        ),
        (
            Vec2::new(-150.0, 302.0),
            Vec2::new(0.0, 256.0),
            10.0,
            Vec2::new(-180.0, 160.0),
            &[(Vec2::new(529.025, 0.0), Vec2::new(495.0, 5000.0))][..],
        ),
        (
            Vec2::new(-150.0, 301.6),
            Vec2::new(0.0, 256.0),
            10.0,
            Vec2::new(-300.0, 301.6),
            &[
                (Vec2::new(529.025, 0.0), Vec2::new(495.0, 5000.0)),
                (Vec2::new(502.0, 267.25), Vec2::new(522.0, 0.25)),
                (Vec2::new(502.0, 335.875), Vec2::new(522.0, 0.125)),
                (Vec2::new(502.0, 209.0), Vec2::new(522.0, 5.0)),
            ][..],
        ),
        (
            Vec2::new(-22.0, 302.8),
            Vec2::new(128.0, 256.0),
            10.0,
            Vec2::new(-300.0, 302.8),
            &[
                (Vec2::new(0.0, 267.35), Vec2::new(5000.0, 0.35)),
                (Vec2::new(0.0, 337.8), Vec2::new(5000.0, 0.2)),
            ][..],
        ),
    ] {
        let mut app = common::sim_app_with_gather();
        let swarm = common::spawn_swarm_at(&mut app, home);
        let worker = common::spawn_worker_at(&mut app, home);
        let deposit = common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: deposit_position,
                amount: 1,
                capacity: 1,
                radius,
            },
        );
        let source = common::spawn_stockpile(&mut app, source_position, 0, 10);
        app.world_mut()
            .entity_mut(source)
            .insert((OwnerSwarm(swarm), StockpileRole::Source));
        let terrain: Vec<_> = rocks
            .iter()
            .map(|&(center, half)| {
                let rock = RockFormation::Rectangle { half };
                let transform = Transform::from_translation(center.extend(0.0));
                app.world_mut().spawn((rock, transform));
                rock.obstacle(&transform)
            })
            .collect();
        let map = SwarmReachabilityMap::new(
            app.world().resource::<IntentGrid>(),
            terrain,
            [(SwarmId::PLAYER, home)],
        )
        .unwrap();
        let region =
            InteractionRegion::deposit(app.world().get::<Transform>(deposit).unwrap(), radius)
                .within_cell(IVec2::ZERO);
        assert!(
            map.interaction_is_reachable(SwarmId::PLAYER, region),
            "the permanent approach should be reachable from {home:?}"
        );
        app.insert_resource(map.clone());
        {
            let mut tuning = app.world_mut().resource_mut::<Tuning>();
            tuning.logistics.material_work_ticks = 1;
            tuning.logistics.bot_speed = 10.0;
        }
        assert!(
            apply_reachable_intent(
                &mut app.world_mut().resource_mut::<IntentGrid>(),
                &map,
                SwarmId::PLAYER,
                IVec2::ZERO,
                IntentKind::Gather,
                IntentWriteAction::Paint,
            )
            .unwrap()
        );
        for _ in 0..450 {
            app.update();
            if app.world().get::<ResourceDeposit>(deposit).unwrap().amount == 0 {
                break;
            }
        }

        let position = app
            .world()
            .get::<Transform>(worker)
            .unwrap()
            .translation
            .truncate();
        assert_eq!(
            app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
            0,
            "worker from {home:?} reached {position:?} without harvesting the deposit at {deposit_position:?} with radius {radius}, assignment: {:?}, order: {:?}",
            app.world().get::<GatherAssignment>(worker),
            app.world().get::<DirectMovementComponent>(worker),
        );
        assert_eq!(world_to_cell(position), IVec2::ZERO);
        assert!(region.contains(position));
    }
}
