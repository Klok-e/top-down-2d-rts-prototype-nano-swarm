use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{GatherAssignment, NanobotType, PopulationDemand, PopulationDemandPlugin, SwarmId},
    navigation::Navigation,
    navigation_runtime::NavigationBudget,
    resources::ResourceDeposit,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn pending_worker_access_preserves_population_demand_until_budget_allows_assignment() {
    let mut app = common::sim_app_with_gather();
    app.add_plugins(PopulationDemandPlugin);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let worker = common::spawn_worker_at(&mut app, Vec2::ZERO);
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(300.0, 100.0),
            amount: 100,
            capacity: 100,
            radius: 48.0,
        },
    );
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::ZERO,
        IntentKind::Gather,
        Some(SwarmId::PLAYER),
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..12 {
        app.update();
    }
    assert!(app.world().get::<GatherAssignment>(worker).is_none());
    assert_eq!(
        app.world().get::<ResourceDeposit>(deposit).unwrap().amount,
        100
    );
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        1
    );
    assert!(app.world().resource::<Navigation>().work().pending > 0);
    assert_eq!(app.world().resource::<Navigation>().work().work, 0);
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32_768;
    for _ in 0..120 {
        app.update();
        if app.world().get::<GatherAssignment>(worker).is_some() {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<GatherAssignment>(worker)
            .map(|a| a.deposit),
        Some(deposit)
    );
}

#[test]
fn pending_placement_waits_for_shared_navigation_work_before_reserving_a_plan() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::PlannedStructure;
    let mut app = common::sim_app_with_planned();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_facility_at(&mut app, swarm, common::cell_world_center(IVec2::new(1, 0)));
    common::spawn_worker_at(&mut app, Vec2::ZERO);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::new(1, 0),
        IntentKind::Build,
        Some(SwarmId::PLAYER),
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..12 {
        app.update();
    }
    let world = app.world_mut();
    assert_eq!(world.query::<&PlannedStructure>().iter(world).count(), 0);
    assert!(world.resource::<Navigation>().work().pending > 0);
    assert_eq!(world.resource::<Navigation>().work().work, 0);
    world.resource_mut::<NavigationBudget>().0 = 32_768;
    let mut planned = false;
    for _ in 0..120 {
        app.update();
        let world = app.world_mut();
        planned = world.query::<&PlannedStructure>().iter(world).count() > 0;
        if planned {
            break;
        }
    }
    assert!(
        planned,
        "a reachable construction site must be committed once its queued check finishes"
    );
}
