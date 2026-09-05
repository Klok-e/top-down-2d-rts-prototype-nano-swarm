use bevy::{ecs::system::RunSystemOnce, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentGrid,
    nanobot::{SwarmId, construction_access::ConstructionAccess},
    navigation::{AccessStatus, Navigation, RouteOutcome},
    physical_world::{PhysicalGeometry, PhysicalWorld},
    terrain::RockFormation,
};

#[path = "../common/mod.rs"]
mod common;

fn geometry(physical: PhysicalWorld) -> PhysicalGeometry {
    physical.snapshot()
}

#[test]
fn permanent_rock_blocks_body_placement_and_routes_detour_around_its_ends() {
    let mut app = common::minimal_app();
    app.world_mut().spawn((
        RockFormation::Rectangle {
            half: Vec2::new(36.0, 600.0),
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.update();
    let physical = app.world_mut().run_system_once(geometry).unwrap();
    assert!(!physical.can_occupy(Vec2::ZERO));
    assert!(!physical.movement_clear(Vec2::new(-500.0, 0.0), Vec2::new(500.0, 0.0)));
    assert!(physical.can_occupy(Vec2::new(0.0, 640.0)));
    let navigation = app.world().resource::<Navigation>();
    let grid = app.world().resource::<IntentGrid>();
    let RouteOutcome::Found(route) = navigation.route(
        Vec2::new(-500.0, 0.0),
        Vec2::new(500.0, 0.0),
        grid,
        SwarmId::PLAYER,
        false,
    ) else {
        panic!("the rock has two open ends")
    };
    assert!(route.waypoints.iter().any(|point| point.y.abs() >= 634.0));
    assert!(
        route.cost > 1600.0,
        "route must include the detour: {}",
        route.cost
    );
}

fn construction_status(
    access: ConstructionAccess,
    grid: Res<IntentGrid>,
    navigation: Res<Navigation>,
) -> (AccessStatus, AccessStatus) {
    let layout = access.snapshot();
    let check = |position: Vec2| {
        layout.check(
            &navigation,
            &grid,
            SwarmId::PLAYER,
            &Transform::from_translation(position.extend(0.0)),
            None,
            Some(Vec2::new(-300.0, 0.0)),
        )
    };
    let overlap = check(Vec2::new(60.0, 0.0));
    let mut exterior = check(Vec2::new(-180.0, 0.0));
    for _ in 0..100 {
        if exterior != AccessStatus::Pending {
            break;
        }
        navigation.advance(&grid, 32_768);
        exterior = check(Vec2::new(-180.0, 0.0));
    }
    (overlap, exterior)
}

#[test]
fn construction_rejects_overlap_with_circular_rock_but_allows_clear_adjacent_ground() {
    let mut app = common::minimal_app();
    app.world_mut().spawn((
        RockFormation::Circle { radius: 100.0 },
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.update();
    let (overlap, exterior) = app
        .world_mut()
        .run_system_once(construction_status)
        .unwrap();
    assert_eq!(overlap, AccessStatus::Rejected);
    assert_eq!(exterior, AccessStatus::Accepted);
}
