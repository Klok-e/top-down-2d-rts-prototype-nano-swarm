use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentGrid,
    nanobot::SwarmId,
    navigation::{Navigation, RouteOutcome},
    scenario::{cell_origin, default_rock_geometry},
};

fn navigation() -> (IntentGrid, Navigation) {
    let grid = IntentGrid::new(64, 64);
    let rocks = default_rock_geometry()
        .into_iter()
        .map(|(rock, transform)| rock.obstacle(&transform))
        .collect();
    let navigation = Navigation::new(&grid, rocks);
    (grid, navigation)
}

fn route_length(navigation: &Navigation, grid: &IntentGrid, start: Vec2, end: Vec2) -> f32 {
    let RouteOutcome::Found(route) = navigation.route(start, end, grid, SwarmId::PLAYER, false)
    else {
        panic!("expected traversable route from {start:?} to {end:?}");
    };
    let mut previous = start;
    let mut distance = 0.0;
    for point in route.waypoints {
        assert!(
            navigation.segment_clear(previous, point),
            "route clips rock at {point:?}"
        );
        distance += previous.distance(point);
        previous = point;
    }
    assert!(
        previous.distance(end) < 1.0,
        "route must reach its destination"
    );
    distance
}

#[test]
fn default_map_central_trip_takes_about_one_minute_and_flanks_are_longer() {
    let (grid, navigation) = navigation();
    let player = Vec2::new(256.0, 256.0);
    let opponent = Vec2::new(12544.0, 12544.0);
    let central = route_length(&navigation, &grid, player, opponent);
    let seconds = central / 300.0;
    assert!(
        (55.0..=65.0).contains(&seconds),
        "central trip is {seconds:.2}s"
    );
    for contested in [Vec2::new(3328.0, 9472.0), Vec2::new(9472.0, 3328.0)] {
        let from_player = route_length(&navigation, &grid, player, contested);
        let from_opponent = route_length(&navigation, &grid, opponent, contested);
        eprintln!(
            "central={seconds:.2}s contested={contested:?} approaches={:.2}s/{:.2}s",
            from_player / 300.0,
            from_opponent / 300.0
        );
        assert!(
            from_player + from_opponent > central * 1.1,
            "flanks must require a longer trip"
        );
        assert!(
            (from_player - from_opponent).abs() / 300.0 < 5.0,
            "contested minerals need comparable approach times"
        );
    }
}

#[test]
fn default_map_rocks_enclose_symmetric_connected_mineral_pockets() {
    let (grid, navigation) = navigation();
    let center = Vec2::splat(6400.0);
    for y in -8..104 {
        for x in -8..104 {
            let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * 128.0 + Vec2::splat(256.0);
            assert_eq!(
                navigation.point_clear(point),
                navigation.point_clear(2.0 * center - point),
                "terrain symmetry at {point:?}"
            );
        }
    }
    for cell in [
        IVec2::new(-1, -1),
        IVec2::new(0, 7),
        IVec2::new(6, 18),
        IVec2::new(25, 25),
        IVec2::new(24, 17),
        IVec2::new(18, 6),
    ] {
        route_length(&navigation, &grid, Vec2::splat(256.0), cell_origin(cell));
    }
    assert!(
        matches!(
            navigation.route(
                Vec2::splat(256.0),
                Vec2::new(-1500.0, 256.0),
                &grid,
                SwarmId::PLAYER,
                false
            ),
            RouteOutcome::Unreachable
        ),
        "outer rock must prevent bypassing the map"
    );
}

#[path = "../common/mod.rs"]
mod common;

#[test]
fn default_map_nanobot_reaches_other_base_in_about_one_minute_of_simulation() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent;
    let mut app = common::sim_app_with_movement();
    app.insert_resource(IntentGrid::new(64, 64));
    for (rock, transform) in default_rock_geometry() {
        app.world_mut().spawn((rock, transform));
    }
    let bot = common::spawn_worker_at(&mut app, Vec2::splat(256.0));
    let goal = Vec2::splat(12544.0);
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: goal,
            stop_radius: 2.0,
        });
    for tick in 1..=3900 {
        app.update();
        let position = app
            .world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate();
        if position.distance(goal) <= 2.01 {
            let seconds = tick as f32 / 60.0;
            eprintln!("actual unobstructed Nanobot traversal={seconds:.2}s");
            assert!(
                seconds >= 55.0,
                "map should require a meaningful one-minute trip"
            );
            return;
        }
    }
    panic!(
        "Nanobot did not reach opposite base within 65 simulated seconds: {:?}",
        app.world().get::<Transform>(bot).unwrap().translation
    );
}

#[test]
fn default_map_base_entrances_have_two_separate_lanes_with_body_clearance() {
    let (_, navigation) = navigation();
    for offset in [-100.0, 100.0] {
        let side_start = Vec2::new(256.0 + offset, 1024.0);
        let side_end = Vec2::new(256.0 + offset, 2048.0);
        let lateral = Vec2::new(offset, -offset);
        let main_start = Vec2::splat(1024.0) + lateral;
        let main_end = Vec2::splat(1792.0) + lateral;
        for (start, end) in [(side_start, side_end), (main_start, main_end)] {
            assert!(
                navigation.segment_clear(start, end),
                "player entrance lacks two-way clearance"
            );
            assert!(
                navigation.segment_clear(Vec2::splat(12800.0) - start, Vec2::splat(12800.0) - end),
                "opponent entrance lacks two-way clearance"
            );
        }
    }
}
