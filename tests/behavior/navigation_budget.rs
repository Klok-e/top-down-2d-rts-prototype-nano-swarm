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
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
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
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(1, 0),
        IntentKind::Build,
        SwarmId::PLAYER,
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

#[test]
fn navigation_work_distinguishes_cold_connectivity_from_warm_route_search() {
    use top_down_2d_rts_prototype_nano_swarm::navigation::{
        Obstacle, RouteGoal, RoutePriority, RouteStatus,
    };
    let grid = IntentGrid::new(4, 4);
    let navigation = Navigation::new(
        &grid,
        vec![Obstacle::Rectangle {
            center: Vec2::ZERO,
            half: Vec2::new(72.0, 144.0),
        }],
    );
    let mut costs = Vec::new();
    for cold in [true, false] {
        let id = navigation.request(
            Vec2::new(-252.0, 36.0),
            RouteGoal::Point(Vec2::new(252.0, 36.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Routine,
        );
        let mut hierarchy_cells = 0;
        let mut coarse = 0;
        let mut fine = 0;
        let mut completed = false;
        for _ in 0..2000 {
            let work = navigation.advance(&grid, 100);
            hierarchy_cells += work.hierarchy_cells;
            coarse += work.coarse_expansions;
            fine += work.fine_expansions;
            if let RouteStatus::Found(route) = navigation.poll(id) {
                assert!(route.cost > 504.0, "the obstacle requires a detour");
                costs.push(route.cost);
                completed = true;
                break;
            }
        }
        assert!(
            completed,
            "detour must finish within the finite work allowance"
        );
        if cold {
            assert!(hierarchy_cells > 0);
        } else {
            assert_eq!(
                hierarchy_cells, 0,
                "unchanged physical connectivity must be reused"
            );
        }
        assert!(
            coarse > 0 && fine > 0,
            "both routing layers must be measured"
        );
        navigation.cancel(id);
    }
    assert!((costs[0] - costs[1]).abs() < 0.01);
}

#[test]
fn simultaneous_cold_routes_share_connectivity_work() {
    use top_down_2d_rts_prototype_nano_swarm::navigation::{
        Obstacle, RouteGoal, RoutePriority, RouteStatus,
    };
    let grid = IntentGrid::new(4, 4);
    let navigation = Navigation::new(
        &grid,
        vec![Obstacle::Rectangle {
            center: Vec2::ZERO,
            half: Vec2::new(72.0, 144.0),
        }],
    );
    let requests: Vec<_> = (0..32)
        .map(|_| {
            navigation.request(
                Vec2::new(-252.0, 36.0),
                RouteGoal::Point(Vec2::new(252.0, 36.0)),
                SwarmId::PLAYER,
                false,
                RoutePriority::Routine,
            )
        })
        .collect();
    let mut hierarchy_cells = 0;
    for _ in 0..1000 {
        let work = navigation.advance(&grid, 1000);
        assert!(work.work <= 1000);
        hierarchy_cells += work.hierarchy_cells;
        if work.pending == 0 {
            break;
        }
    }
    assert!(
        hierarchy_cells <= 900,
        "the 2048-unit square holds fewer than 900 navigation cells, but shared cold routes visited {hierarchy_cells}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|&&id| matches!(navigation.poll(id), RouteStatus::Found(_)))
            .count(),
        32,
        "concurrent routes through the same cold layout must eventually finish"
    );
    for id in requests {
        let RouteStatus::Found(route) = navigation.poll(id) else {
            unreachable!()
        };
        let mut previous = Vec2::new(-252.0, 36.0);
        for point in route.waypoints {
            assert!(navigation.segment_clear(previous, point));
            previous = point;
        }
        assert_eq!(previous, Vec2::new(252.0, 36.0));
    }
}

#[test]
fn clearing_preempts_a_pending_detour_without_starving_its_aged_completion() {
    use top_down_2d_rts_prototype_nano_swarm::navigation::{
        Obstacle, RouteGoal, RoutePriority, RouteStatus,
    };
    let grid = IntentGrid::new(4, 4);
    let navigation = Navigation::new(
        &grid,
        vec![Obstacle::Rectangle {
            center: Vec2::ZERO,
            half: Vec2::new(72.0, 144.0),
        }],
    );
    let routine = navigation.request(
        Vec2::new(-252.0, 36.0),
        RouteGoal::Point(Vec2::new(252.0, 36.0)),
        SwarmId::PLAYER,
        false,
        RoutePriority::Routine,
    );
    navigation.advance(&grid, 1);
    assert!(matches!(navigation.poll(routine), RouteStatus::Pending));
    let clearing = navigation.request(
        Vec2::new(-500.0, -500.0),
        RouteGoal::Point(Vec2::new(-400.0, -500.0)),
        SwarmId::PLAYER,
        false,
        RoutePriority::Clearing,
    );
    let first = navigation.advance(&grid, 8);
    assert!(first.work <= 8);
    assert!(
        matches!(navigation.poll(clearing), RouteStatus::Found(_)),
        "new clearing request must preempt the unfinished detour"
    );
    assert!(matches!(navigation.poll(routine), RouteStatus::Pending));
    navigation.cancel(clearing);
    for _ in 0..1000 {
        let urgent = navigation.request(
            Vec2::new(-500.0, -500.0),
            RouteGoal::Point(Vec2::new(-400.0, -500.0)),
            SwarmId::PLAYER,
            false,
            RoutePriority::Clearing,
        );
        assert!(navigation.advance(&grid, 100).work <= 100);
        navigation.cancel(urgent);
        if matches!(navigation.poll(routine), RouteStatus::Found(_)) {
            break;
        }
    }
    let RouteStatus::Found(route) = navigation.poll(routine) else {
        panic!("aged detour starved under finite urgent arrivals")
    };
    let mut previous = Vec2::new(-252.0, 36.0);
    for point in route.waypoints {
        assert!(navigation.segment_clear(previous, point));
        previous = point;
    }
    assert_eq!(previous, Vec2::new(252.0, 36.0));
}

#[test]
fn partial_connectivity_build_cannot_publish_a_route_through_changed_obstacles() {
    use top_down_2d_rts_prototype_nano_swarm::navigation::{
        Obstacle, RouteGoal, RoutePriority, RouteStatus,
    };
    let grid = IntentGrid::new(4, 4);
    let partial_wall = Obstacle::Rectangle {
        center: Vec2::ZERO,
        half: Vec2::new(72.0, 144.0),
    };
    let mut navigation = Navigation::new(&grid, vec![partial_wall]);
    let request = navigation.request(
        Vec2::new(-252.0, 36.0),
        RouteGoal::Point(Vec2::new(252.0, 36.0)),
        SwarmId::PLAYER,
        false,
        RoutePriority::Routine,
    );
    assert_eq!(navigation.advance(&grid, 100).work, 100);
    assert!(matches!(navigation.poll(request), RouteStatus::Pending));
    navigation.refresh(
        &grid,
        vec![Obstacle::Rectangle {
            center: Vec2::ZERO,
            half: Vec2::new(72.0, 1100.0),
        }],
    );
    for _ in 0..1000 {
        assert!(navigation.advance(&grid, 1000).work <= 1000);
        if !matches!(navigation.poll(request), RouteStatus::Pending) {
            break;
        }
    }
    assert!(
        matches!(navigation.poll(request), RouteStatus::Unreachable),
        "a stale partial build must not reopen the sealed wall"
    );
    navigation.refresh(&grid, vec![partial_wall]);
    for _ in 0..1000 {
        assert!(navigation.advance(&grid, 1000).work <= 1000);
        if !matches!(navigation.poll(request), RouteStatus::Pending) {
            break;
        }
    }
    let RouteStatus::Found(route) = navigation.poll(request) else {
        panic!("removing the seal must restore the detour")
    };
    let mut previous = Vec2::new(-252.0, 36.0);
    for point in route.waypoints {
        assert!(navigation.segment_clear(previous, point));
        previous = point;
    }
    assert_eq!(previous, Vec2::new(252.0, 36.0));
}

#[test]
fn connectivity_refresh_replaces_pending_geometry_and_caches_each_final_answer() {
    use top_down_2d_rts_prototype_nano_swarm::navigation::{
        ConnectivityStatus, Obstacle, RouteGoal,
    };

    let grid = IntentGrid::new(4, 4);
    let partial_wall = Obstacle::Rectangle {
        center: Vec2::ZERO,
        half: Vec2::new(72.0, 144.0),
    };
    let sealing_wall = Obstacle::Rectangle {
        center: Vec2::ZERO,
        half: Vec2::new(72.0, 1_100.0),
    };
    let start = Vec2::new(-252.0, 36.0);
    let goal = RouteGoal::Point(Vec2::new(252.0, 36.0));
    let mut navigation = Navigation::new(&grid, vec![partial_wall]);

    assert_eq!(
        navigation.query_connectivity(start, goal),
        ConnectivityStatus::Pending
    );
    assert_eq!(navigation.advance(&grid, 1).work, 1);
    assert_eq!(
        navigation.query_connectivity(start, goal),
        ConnectivityStatus::Pending
    );

    navigation.refresh(&grid, vec![sealing_wall]);
    assert_eq!(
        navigation.query_connectivity(start, goal),
        ConnectivityStatus::Pending,
        "refresh must invalidate a partial connectivity build immediately"
    );
    let mut status = ConnectivityStatus::Pending;
    for _ in 0..1_000 {
        assert!(navigation.advance(&grid, 1_000).work <= 1_000);
        status = navigation.query_connectivity(start, goal);
        if status != ConnectivityStatus::Pending {
            break;
        }
    }
    assert_eq!(status, ConnectivityStatus::Unreachable);
    assert_eq!(
        navigation.query_connectivity(start, goal),
        ConnectivityStatus::Unreachable,
        "the completed sealed-world answer must remain cached"
    );

    navigation.refresh(&grid, vec![partial_wall]);
    assert_eq!(
        navigation.query_connectivity(start, goal),
        ConnectivityStatus::Pending,
        "refresh must invalidate the cached sealed-world answer immediately"
    );
    for _ in 0..1_000 {
        assert!(navigation.advance(&grid, 1_000).work <= 1_000);
        status = navigation.query_connectivity(start, goal);
        if status != ConnectivityStatus::Pending {
            break;
        }
    }
    let connected = ConnectivityStatus::Connected {
        endpoint: Vec2::new(252.0, 36.0),
    };
    assert_eq!(status, connected);
    assert_eq!(
        navigation.query_connectivity(start, goal),
        connected,
        "the completed partial-wall answer must remain cached"
    );
}
