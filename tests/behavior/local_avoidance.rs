use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent;
#[path = "../common/mod.rs"]
mod common;

#[test]
fn opposing_bodies_pass_without_swept_overlap_in_open_space() {
    let mut app = common::sim_app_with_movement();
    let a = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    let b = common::spawn_worker_at(&mut app, Vec2::new(600., 300.));
    for (entity, xy) in [(a, Vec2::new(600., 300.)), (b, Vec2::new(200., 300.))] {
        app.world_mut()
            .entity_mut(entity)
            .insert(DirectMovementComponent {
                xy,
                stop_radius: 0.,
                interaction: None,
                speed: None,
            });
    }
    let mut previous = [Vec2::new(200., 300.), Vec2::new(600., 300.)];
    for _ in 0..500 {
        app.update();
        let now = [a, b].map(|entity| {
            app.world()
                .get::<Transform>(entity)
                .unwrap()
                .translation
                .truncate()
        });
        let relative = previous[0] - previous[1];
        let travel = (now[0] - previous[0]) - (now[1] - previous[1]);
        let t = if travel.length_squared() > 0. {
            (-relative.dot(travel) / travel.length_squared()).clamp(0., 1.)
        } else {
            0.
        };
        assert!(
            (relative + travel * t).length() >= 67.99,
            "body collision: {previous:?} -> {now:?}"
        );
        previous = now;
    }
    assert!(
        previous[0].distance(Vec2::new(600., 300.)) < 3.,
        "first stopped at {:?}",
        previous[0]
    );
    assert!(
        previous[1].distance(Vec2::new(200., 300.)) < 3.,
        "second stopped at {:?}",
        previous[1]
    );
}

#[test]
fn opposing_traffic_backs_out_of_one_cell_passage_and_resumes() {
    let mut app = common::sim_app_with_movement();
    for y in [252., 396.] {
        let wall = common::spawn_structure_at(&mut app, Vec2::new(432., y));
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(4.5, 1.125, 1.);
    }
    let a = common::spawn_worker_at(&mut app, Vec2::new(360., 324.));
    let b = common::spawn_worker_at(&mut app, Vec2::new(504., 324.));
    for (entity, xy) in [(a, Vec2::new(900., 324.)), (b, Vec2::new(180., 324.))] {
        app.world_mut()
            .entity_mut(entity)
            .insert(DirectMovementComponent {
                xy,
                stop_radius: 0.,
                interaction: None,
                speed: None,
            });
    }
    let mut previous = [Vec2::new(360., 324.), Vec2::new(504., 324.)];
    let mut backed_out = false;
    let mut searches = 0;
    for _ in 0..700 {
        app.update();
        searches += app
            .world()
            .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
            .work()
            .completed;
        let now = [a, b].map(|entity| {
            app.world()
                .get::<Transform>(entity)
                .unwrap()
                .translation
                .truncate()
        });
        let relative = previous[0] - previous[1];
        let travel = (now[0] - previous[0]) - (now[1] - previous[1]);
        let t = if travel.length_squared() > 0. {
            (-relative.dot(travel) / travel.length_squared()).clamp(0., 1.)
        } else {
            0.
        };
        assert!(
            (relative + travel * t).length() >= 67.99,
            "body collision: {previous:?} -> {now:?}"
        );
        for index in 0..2 {
            assert!(
                app.world()
                    .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
                    .segment_clear(previous[index], now[index]),
                "wall clipping: {previous:?} -> {now:?}"
            );
        }
        backed_out |= now[0].x < 253. || now[1].x > 611.;
        previous = now;
    }
    assert_eq!(searches, 2, "traffic must retain its original routes");
    assert!(backed_out, "yielding bot never left passage: {previous:?}");
    assert!(
        previous[0].distance(Vec2::new(900., 324.)) < 3.,
        "first stopped at {:?}",
        previous[0]
    );
    assert!(
        previous[1].distance(Vec2::new(180., 324.)) < 3.,
        "second stopped at {:?}",
        previous[1]
    );
}

#[test]
fn trapped_opposing_traffic_waits_without_overlap_or_wall_crossing() {
    let mut app = common::sim_app_with_movement();
    for (center, scale) in [
        (Vec2::new(432., 252.), Vec3::new(4.5, 1.125, 1.)),
        (Vec2::new(432., 396.), Vec3::new(4.5, 1.125, 1.)),
        (Vec2::new(252., 324.), Vec3::new(1.125, 3.375, 1.)),
        (Vec2::new(612., 324.), Vec3::new(1.125, 3.375, 1.)),
    ] {
        let wall = common::spawn_structure_at(&mut app, center);
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = scale;
    }
    let a = common::spawn_worker_at(&mut app, Vec2::new(360., 324.));
    let b = common::spawn_worker_at(&mut app, Vec2::new(504., 324.));
    for (entity, xy) in [(a, Vec2::new(504., 324.)), (b, Vec2::new(360., 324.))] {
        app.world_mut()
            .entity_mut(entity)
            .insert(DirectMovementComponent {
                xy,
                stop_radius: 0.,
                interaction: None,
                speed: None,
            });
    }
    let mut previous = [Vec2::new(360., 324.), Vec2::new(504., 324.)];
    for _ in 0..300 {
        app.update();
        let now = [a, b].map(|entity| {
            app.world()
                .get::<Transform>(entity)
                .unwrap()
                .translation
                .truncate()
        });
        let relative = previous[0] - previous[1];
        let travel = (now[0] - previous[0]) - (now[1] - previous[1]);
        let t = if travel.length_squared() > 0. {
            (-relative.dot(travel) / travel.length_squared()).clamp(0., 1.)
        } else {
            0.
        };
        assert!(
            (relative + travel * t).length() >= 67.99,
            "body collision: {previous:?} -> {now:?}"
        );
        for index in 0..2 {
            assert!(
                app.world()
                    .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
                    .segment_clear(previous[index], now[index])
            );
        }
        previous = now;
    }
    assert!(
        previous[0].x < previous[1].x,
        "bots crossed in sealed one-cell passage"
    );
    assert!(app.world().get::<DirectMovementComponent>(a).is_some());
    assert!(app.world().get::<DirectMovementComponent>(b).is_some());
}

#[test]
fn wider_lanes_allow_side_by_side_passing_without_deflection() {
    let mut app = common::sim_app_with_movement();
    let a = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    let b = common::spawn_worker_at(&mut app, Vec2::new(600., 372.));
    for (entity, xy) in [(a, Vec2::new(600., 300.)), (b, Vec2::new(200., 372.))] {
        app.world_mut()
            .entity_mut(entity)
            .insert(DirectMovementComponent {
                xy,
                stop_radius: 0.,
                interaction: None,
                speed: None,
            });
    }
    for _ in 0..100 {
        app.update();
        let first = app.world().get::<Transform>(a).unwrap().translation;
        let second = app.world().get::<Transform>(b).unwrap().translation;
        assert!(
            (first.y - 300.).abs() < 0.001 && (second.y - 372.).abs() < 0.001,
            "clear lanes should not yield: {first:?}, {second:?}"
        );
    }
    assert!((app.world().get::<Transform>(a).unwrap().translation.x - 600.).abs() < 3.);
    assert!((app.world().get::<Transform>(b).unwrap().translation.x - 200.).abs() < 3.);
}

#[test]
fn following_traffic_yields_with_the_front_bot_in_a_bottleneck() {
    for spawn_order in [[0, 1, 2, 3], [1, 3, 0, 2], [3, 2, 1, 0]] {
        let mut app = common::sim_app_with_movement();
        for y in [252., 396.] {
            let wall = common::spawn_structure_at(&mut app, Vec2::new(432., y));
            app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(4.5, 1.125, 1.);
        }
        let starts = [
            Vec2::new(340., 324.),
            Vec2::new(412., 324.),
            Vec2::new(484., 324.),
            Vec2::new(556., 324.),
        ];
        let goals = [
            Vec2::new(828., 324.),
            Vec2::new(900., 324.),
            Vec2::new(180., 324.),
            Vec2::new(108., 324.),
        ];
        let mut bots = [Entity::PLACEHOLDER; 4];
        for index in spawn_order {
            bots[index] = common::spawn_worker_at(&mut app, starts[index]);
        }
        for (entity, xy) in bots.into_iter().zip(goals) {
            app.world_mut()
                .entity_mut(entity)
                .insert(DirectMovementComponent {
                    xy,
                    stop_radius: 0.,
                    interaction: None,
                    speed: None,
                });
        }
        let mut previous = starts;
        for _ in 0..1500 {
            app.update();
            let now = bots.map(|entity| {
                app.world()
                    .get::<Transform>(entity)
                    .unwrap()
                    .translation
                    .truncate()
            });
            for first in 0..4 {
                assert!(
                    app.world()
                        .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
                        .segment_clear(previous[first], now[first])
                );
                for second in first + 1..4 {
                    let relative = previous[first] - previous[second];
                    let travel = now[first] - previous[first] - (now[second] - previous[second]);
                    let t = if travel.length_squared() > 0. {
                        (-relative.dot(travel) / travel.length_squared()).clamp(0., 1.)
                    } else {
                        0.
                    };
                    assert!(
                        (relative + travel * t).length() >= 67.99,
                        "stream collision: {previous:?} -> {now:?}"
                    );
                }
            }
            previous = now;
        }
        for index in 0..4 {
            assert!(
                previous[index].distance(goals[index]) < 3.,
                "stream stalled: {previous:?}"
            );
        }
    }
}

#[test]
fn queued_movement_waits_safely_and_retries_after_an_obstacle_is_removed() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_movement();
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    let bot = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(600., 300.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate(),
        Vec2::new(200., 300.)
    );
    assert!(
        app.world().get::<DirectMovementComponent>(bot).is_some(),
        "pending navigation must retain work"
    );
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32768;
    for _ in 0..10 {
        app.update();
    }
    let before = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(before.x > 200.);
    let wall = common::spawn_structure_at(&mut app, Vec2::new(400., 300.));
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate(),
        before,
        "obstruction stops the old route before queued replacement"
    );
    app.world_mut().despawn(wall);
    app.world_mut().resource_mut::<NavigationBudget>().0 = 32768;
    for _ in 0..100 {
        app.update();
    }
    assert!((app.world().get::<Transform>(bot).unwrap().translation.x - 600.).abs() < 3.);
}

#[test]
fn moving_destination_does_not_starve_a_pending_pursuit_route() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_movement();
    app.world_mut().resource_mut::<NavigationBudget>().0 = 256;
    let wall = common::spawn_structure_at(&mut app, Vec2::new(432., 324.));
    app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(1.125, 3.375, 1.);
    let bot = common::spawn_worker_at(&mut app, Vec2::new(200., 324.));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(700., 324.),
            stop_radius: 40.,
            interaction: None,
            speed: None,
        });
    for tick in 0..1000 {
        if let Some(mut order) = app.world_mut().get_mut::<DirectMovementComponent>(bot) {
            order.xy.y = 324. + (tick % 20) as f32;
        }
        app.update();
    }
    let position = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        position.x > 500.,
        "continuously moving target starved pursuit: {position:?}"
    );
}
