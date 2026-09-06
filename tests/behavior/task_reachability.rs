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
fn pending_recovery_waits_but_proven_disconnected_material_causes_collapse() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{OwnerSwarm, ProductionCollapseState};
    let mut app = common::sim_app_with_collapse();
    app.insert_resource(IntentGrid::new(2, 2));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    let swarm = common::spawn_swarm_at(&mut app, Vec2::new(-200.0, 100.0));
    common::spawn_worker_at(&mut app, Vec2::new(-200.0, 100.0));
    common::spawn_hauler_at(&mut app, Vec2::new(-200.0, 150.0));
    let material = common::spawn_sink_stockpile(&mut app, Vec2::new(-200.0, -100.0), 50, 100);
    app.world_mut()
        .entity_mut(material)
        .insert(OwnerSwarm(swarm));
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::new(200.0, 100.0));
    app.world_mut()
        .get_mut::<top_down_2d_rts_prototype_nano_swarm::nanobot::ProductionFacility>(facility)
        .unwrap()
        .input_amount = 0;
    let wall = common::spawn_structure_at(&mut app, Vec2::ZERO);
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.0, 40.0, 1.0);
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..5 {
        app.update();
    }
    assert!(
        !app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "navigation delay cannot conclude the match"
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32_768;
    for _ in 0..60 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "material isolated from production is not a recovery path: {:?}",
        app.world()
            .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
            .work()
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
    app.update();
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
