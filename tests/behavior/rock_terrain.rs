use bevy::{ecs::system::RunSystemOnce, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentGrid,
    nanobot::{SwarmId, construction_access::ConstructionAccess},
    navigation::{AccessStatus, BODY_RADIUS, Navigation, Obstacle, RouteOutcome},
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

fn spawn_default_rocks(app: &mut App) -> Vec<(RockFormation, Transform)> {
    app.insert_resource(IntentGrid::new(
        top_down_2d_rts_prototype_nano_swarm::MAP_WIDTH as i32,
        top_down_2d_rts_prototype_nano_swarm::MAP_HEIGHT as i32,
    ));
    let rocks = top_down_2d_rts_prototype_nano_swarm::scenario::default_rock_geometry();
    assert_eq!(rocks.len(), 347);
    for (rock, transform) in &rocks {
        app.world_mut().spawn((*rock, *transform));
    }
    rocks
}

/// Deterministic scatter over the middle of the map, dense enough to cross many rocks.
fn samples(count: usize) -> Vec<Vec2> {
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / (1u32 << 24) as f32
    };
    (0..count)
        .map(|_| Vec2::new(next() - 0.5, next() - 0.5) * 40_000.0)
        .collect()
}

/// Probes at, just inside, and just beyond body clearance around a rock, where a
/// spatial index that skips too eagerly would disagree with the exact shape test.
fn clearance_probes(shape: Obstacle) -> Vec<Vec2> {
    let (center, half) = match shape {
        Obstacle::Rectangle { center, half } => (center, half),
        Obstacle::Circle { center, radius } => (center, Vec2::splat(radius)),
    };
    let mut probes = Vec::new();
    for offset in [-0.01, 0.0, 0.01, 0.99, 1.01] {
        let reach = half + Vec2::splat(BODY_RADIUS + offset);
        for side in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
            probes.push(center + reach * side);
        }
    }
    probes
}

fn assert_matches_brute_force(
    can_occupy: impl Fn(Vec2) -> bool,
    movement_clear: impl Fn(Vec2, Vec2) -> bool,
    rocks: &[(RockFormation, Transform)],
) {
    let shapes: Vec<_> = rocks
        .iter()
        .map(|(rock, transform)| rock.obstacle(transform))
        .collect();
    let mut points = samples(4000);
    points.extend(shapes.iter().flat_map(|&shape| clearance_probes(shape)));
    let mut blocked_points = 0;
    for point in &points {
        let expected = shapes.iter().all(|shape| shape.admits_body(*point));
        blocked_points += usize::from(!expected);
        assert_eq!(can_occupy(*point), expected, "point {point}");
    }
    assert!(blocked_points > 0, "samples must reach rock");
    let mut blocked_segments = 0;
    for (index, pair) in points.windows(2).enumerate() {
        let start = pair[0];
        // Mix short movement steps with long sweeps.
        let end = if index % 2 == 0 {
            start + (pair[1] - start).normalize_or_zero() * 40.0
        } else {
            pair[1]
        };
        let expected = shapes.iter().all(|shape| shape.segment_clear(start, end));
        blocked_segments += usize::from(!expected);
        assert_eq!(
            movement_clear(start, end),
            expected,
            "segment {start} -> {end}"
        );
    }
    assert!(blocked_segments > 0, "segments must reach rock");
}

#[test]
fn default_map_answers_match_a_brute_force_scan_of_every_rock() {
    let mut app = common::minimal_app();
    let rocks = spawn_default_rocks(&mut app);
    app.update();
    let physical = app.world_mut().run_system_once(geometry).unwrap();
    assert_matches_brute_force(
        |point| physical.can_occupy(point),
        |start, end| physical.movement_clear(start, end),
        &rocks,
    );
}

#[test]
fn navigation_clearance_on_the_default_map_matches_a_brute_force_scan_of_every_rock() {
    let mut app = common::minimal_app();
    let rocks = spawn_default_rocks(&mut app);
    let grid = app.world().resource::<IntentGrid>();
    let navigation = Navigation::new(
        grid,
        rocks
            .iter()
            .map(|(rock, transform)| rock.obstacle(transform))
            .collect(),
    );
    assert_matches_brute_force(
        |point| navigation.point_clear(point),
        |start, end| navigation.segment_clear(start, end),
        &rocks,
    );
}

#[test]
fn answers_follow_the_rocks_of_the_current_session_after_a_restart() {
    let mut app = common::minimal_app();
    let default_rocks = spawn_default_rocks(&mut app);
    let old_center = default_rocks[0].1.translation.truncate();
    app.update();
    let before = app.world_mut().run_system_once(geometry).unwrap();
    assert!(!before.can_occupy(old_center));
    let old_rocks: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<RockFormation>>()
        .iter(app.world())
        .collect();
    for rock in old_rocks {
        app.world_mut().despawn(rock);
    }
    let replacement = vec![(
        RockFormation::Rectangle {
            half: Vec2::new(36.0, 600.0),
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
    )];
    for (rock, transform) in &replacement {
        app.world_mut().spawn((*rock, *transform));
    }
    app.update();
    let physical = app.world_mut().run_system_once(geometry).unwrap();
    assert!(!physical.can_occupy(Vec2::ZERO));
    assert!(physical.can_occupy(old_center));
    assert!(!physical.movement_clear(Vec2::new(-500.0, 0.0), Vec2::new(500.0, 0.0)));
}
