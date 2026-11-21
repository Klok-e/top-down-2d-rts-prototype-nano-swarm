use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{
    CongestionRecovery, DirectMovementComponent, SwarmId, SwarmMember,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn friendly_traffic_recovers_in_a_sealed_single_file_passage() {
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
    let left = common::spawn_worker_at(&mut app, Vec2::new(360., 324.));
    let right = common::spawn_worker_at(&mut app, Vec2::new(504., 324.));
    for (entity, xy) in [
        (left, Vec2::new(504., 324.)),
        (right, Vec2::new(360., 324.)),
    ] {
        app.world_mut().entity_mut(entity).insert((
            SwarmMember(SwarmId::PLAYER),
            DirectMovementComponent {
                xy,
                stop_radius: 0.,
                interaction: None,
                speed: None,
            },
        ));
    }
    let mut overlapped = false;
    let mut previous = [Vec2::new(360., 324.), Vec2::new(504., 324.)];
    for tick in 0..300 {
        app.update();
        let positions = [left, right].map(|entity| {
            app.world()
                .get::<Transform>(entity)
                .unwrap()
                .translation
                .truncate()
        });
        if positions[0].distance(positions[1]) < 67.999 {
            assert!(tick >= 10, "overlap started before one simulation second");
            overlapped = true;
        }
        for i in 0..2 {
            assert!(
                (322.0..=542.0).contains(&positions[i].x)
                    && (322.0..=326.0).contains(&positions[i].y),
                "body crossed chamber walls: {:?}",
                positions[i]
            );
            assert!(
                positions[i].distance(previous[i]) <= 5.001,
                "recovery teleported a body"
            );
        }
        previous = positions;
    }
    assert!(
        overlapped,
        "sealed single-file traffic cannot pass without temporary overlap"
    );
    assert!(app.world().get::<CongestionRecovery>(left).is_none());
    assert!(app.world().get::<CongestionRecovery>(right).is_none());
    let positions = [left, right].map(|entity| {
        app.world()
            .get::<Transform>(entity)
            .unwrap()
            .translation
            .truncate()
    });
    assert!(
        positions[0].distance(Vec2::new(504., 324.)) < 3.
            && positions[1].distance(Vec2::new(360., 324.)) < 3.,
        "friendly traffic never recovered: {positions:?}"
    );
}

#[test]
fn idle_friend_steps_aside_before_approaching_traffic_stalls() {
    let mut app = common::sim_app_with_movement();
    let mover = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    let idle = common::spawn_worker_at(&mut app, Vec2::new(400., 300.));
    app.world_mut()
        .entity_mut(mover)
        .insert(DirectMovementComponent {
            xy: Vec2::new(600., 300.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
    let mut yielded_early = false;
    for _ in 0..150 {
        app.update();
        let a = app
            .world()
            .get::<Transform>(mover)
            .unwrap()
            .translation
            .truncate();
        let b = app
            .world()
            .get::<Transform>(idle)
            .unwrap()
            .translation
            .truncate();
        yielded_early |= b.y > 301. && a.x < 330.;
        assert!(
            a.distance(b) >= 67.999,
            "ordinary yielding overlapped: {a:?}, {b:?}"
        );
    }
    assert!(
        yielded_early,
        "idle friend waited until traffic was blocked before yielding"
    );
    let end = app
        .world()
        .get::<Transform>(mover)
        .unwrap()
        .translation
        .truncate();
    assert!(
        end.distance(Vec2::new(600., 300.)) < 3.,
        "passing traffic stalled at {end:?}"
    );
}

#[test]
fn route_progress_away_from_goal_does_not_count_as_a_traffic_stall() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Commitment, RemainingTravel, VelocityComponent, velocity_system,
    };
    let mut app = common::minimal_app();
    app.add_systems(FixedUpdate, velocity_system);
    let traveller = common::spawn_worker_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .entity_mut(traveller)
        .insert(DirectMovementComponent {
            xy: Vec2::new(400., 0.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
    // The route first runs north around an obstacle: remaining route length falls
    // while Euclidean distance to the eastward destination grows.
    for tick in 0..30 {
        app.world_mut().entity_mut(traveller).insert((
            RemainingTravel(1000. - tick as f32 * 5.),
            VelocityComponent {
                value: Vec2::Y * 5.,
            },
        ));
        app.update();
    }
    let blocker = common::spawn_worker_at(&mut app, Vec2::new(0., 220.));
    app.world_mut()
        .entity_mut(blocker)
        .insert(Commitment::Working);
    for _ in 0..9 {
        app.world_mut().entity_mut(traveller).insert((
            RemainingTravel(850.),
            VelocityComponent {
                value: Vec2::Y * 5.,
            },
        ));
        app.update();
        assert!(
            app.world().get::<CongestionRecovery>(traveller).is_none(),
            "a progressing detour must not accumulate recovery time before first contact"
        );
    }
}

#[test]
fn queued_recovery_separates_from_a_working_body_without_losing_cargo() {
    for offset in [Vec2::ZERO, Vec2::X * 4320.0] {
        check_queued_recovery(offset);
    }
}

fn check_queued_recovery(offset: Vec2) {
    use top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid;
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Cargo, Commitment, RemainingTravel, VelocityComponent, WaitingForWork, WorkBlocked,
        separation_system, velocity_system, work_standing_system,
    };
    use top_down_2d_rts_prototype_nano_swarm::resources::ResourceKind;
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(32, 32));
    app.add_systems(
        FixedUpdate,
        (separation_system, velocity_system, work_standing_system).chain(),
    );
    for y in [252., 396.] {
        let wall = common::spawn_structure_at(&mut app, Vec2::new(432., y) + offset);
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(4.5, 1.125, 1.);
    }
    let worker_position = Vec2::new(432., 324.) + offset;
    let worker = common::spawn_worker_at(&mut app, worker_position);
    app.world_mut()
        .entity_mut(worker)
        .insert(Commitment::Working);
    let loaded = common::spawn_hauler_at(&mut app, Vec2::new(360., 324.) + offset);
    app.world_mut().entity_mut(loaded).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 20,
        },
        DirectMovementComponent {
            xy: Vec2::new(504., 324.) + offset,
            stop_radius: 0.,
            interaction: None,
            speed: None,
        },
    ));
    let mut crossed = false;
    for _ in 0..180 {
        let x = app.world().get::<Transform>(loaded).unwrap().translation.x;
        app.world_mut().entity_mut(loaded).insert((
            RemainingTravel(504. + offset.x - x),
            VelocityComponent {
                value: Vec2::X * 5.,
            },
        ));
        app.update();
        let position = app
            .world()
            .get::<Transform>(loaded)
            .unwrap()
            .translation
            .truncate();
        if position.distance(worker_position) <= 68.001 {
            crossed = true;
            break;
        }
    }
    assert!(
        crossed && app.world().get::<CongestionRecovery>(loaded).is_some(),
        "fixture must enter transit overlap first"
    );
    // Destination capacity disappears during transit, leaving no route steering.
    app.world_mut()
        .entity_mut(loaded)
        .insert(WaitingForWork)
        .remove::<RemainingTravel>();
    for _ in 0..240 {
        app.update();
    }
    let position = app
        .world()
        .get::<Transform>(loaded)
        .unwrap()
        .translation
        .truncate();
    assert!(
        position.distance(worker_position) >= 67.999,
        "queued recovery remained embedded: {position:?}"
    );
    assert!(
        app.world().get::<WorkBlocked>(worker).is_none(),
        "transit must not suspend the worker indefinitely"
    );
    assert!(
        app.world()
            .get::<Transform>(worker)
            .unwrap()
            .translation
            .truncate()
            .distance(worker_position)
            < 0.001
    );
    assert_eq!(app.world().get::<Cargo>(loaded).unwrap().amount, 20);
}

#[test]
fn loaded_worker_can_pass_an_occupied_intermediate_waypoint() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, Commitment},
        navigation::BODY_RADIUS,
        resources::ResourceKind,
    };

    let mut app = common::sim_app_with_movement();
    common::spawn_structure_at(&mut app, Vec2::new(-36., 36.));
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: Vec2::new(96., 96.),
            amount: 50,
            capacity: 50,
            radius: 64.,
        },
    );
    let loading_position = Vec2::new(-51.5, -36.7);
    let blocker = common::spawn_hauler_at(&mut app, loading_position);
    app.world_mut()
        .entity_mut(blocker)
        .insert(Commitment::Working);
    let traveller = common::spawn_worker_at(&mut app, Vec2::new(-22., -110.));
    let destination = Vec2::new(-2., 108.);
    app.world_mut().entity_mut(traveller).insert((
        DirectMovementComponent {
            xy: destination,
            stop_radius: 0.,
            speed: None,
            interaction: None,
        },
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 4,
        },
    ));
    let mut closest_spacing = f32::INFINITY;
    for _ in 0..125 {
        app.update();
        let world = app.world();
        let position = |bot| world.get::<Transform>(bot).unwrap().translation.truncate();
        closest_spacing = closest_spacing.min(position(traveller).distance(position(blocker)));
    }
    let end = app
        .world()
        .get::<Transform>(traveller)
        .unwrap()
        .translation
        .truncate();
    assert!(
        end.distance(destination) <= 2.0
            && app
                .world()
                .get::<DirectMovementComponent>(traveller)
                .is_none(),
        "worker could not proceed around the occupied turn: {end:?}"
    );
    assert!(closest_spacing >= 2.0 * BODY_RADIUS - 0.001);
    assert_eq!(app.world().get::<Cargo>(traveller).unwrap().amount, 4);
}

#[test]
fn loaded_head_on_yield_resumes_when_both_detours_are_blocked() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, RemainingTravel, VelocityComponent, velocity_system},
        navigation::{BODY_RADIUS, Navigation},
        resources::ResourceKind,
    };

    let mut app = common::minimal_app();
    app.add_systems(FixedUpdate, velocity_system);
    for center in [
        Vec2::new(-36., -108.),
        Vec2::new(-36., 36.),
        Vec2::new(108., -36.),
    ] {
        let structure = common::spawn_structure_at(&mut app, center);
        app.world_mut()
            .get_mut::<Transform>(structure)
            .unwrap()
            .scale = Vec3::new(1.125, 1.125, 1.);
    }
    let worker_start = Vec2::new(34.05, -102.78);
    let hauler_start = Vec2::new(32.88, -28.60);
    let worker = common::spawn_worker_at(&mut app, worker_start);
    let hauler = common::spawn_hauler_at(&mut app, hauler_start);
    let routes = [
        (
            worker,
            worker_start,
            Vec2::new(38., 140.),
            Vec2::new(37.5, -37.),
        ),
        (
            hauler,
            hauler_start,
            Vec2::new(39., -140.),
            Vec2::new(36., -36.),
        ),
    ];
    for (bot, _, xy, _) in routes {
        app.world_mut().entity_mut(bot).insert((
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 4,
            },
            DirectMovementComponent {
                xy,
                stop_radius: 0.,
                speed: None,
                interaction: None,
            },
        ));
    }
    let position = |world: &World, bot| world.get::<Transform>(bot).unwrap().translation.truncate();
    for tick in 0..100 {
        for (bot, start, goal, bend) in routes {
            let p = position(app.world(), bot);
            let waypoint = if bot == worker && tick == 0 {
                // Initial alignment toward the aisle changes who yields when the route turns.
                start + Vec2::new(3.5, 4.)
            } else if (bend.y - p.y) * (goal.y - start.y).signum() > 2. {
                bend
            } else {
                goal
            };
            let step = waypoint - p;
            app.world_mut().entity_mut(bot).insert((
                VelocityComponent {
                    value: step.normalize_or_zero() * step.length().min(5.),
                },
                RemainingTravel(p.distance(goal)),
            ));
        }
        app.update();
        let world = app.world();
        let navigation = world.resource::<Navigation>();
        for bot in [worker, hauler] {
            assert!(
                navigation.point_clear(position(world, bot)),
                "bot crossed the aisle wall"
            );
        }
        if tick < 60 {
            assert!(
                position(world, worker).distance(position(world, hauler))
                    >= 2. * BODY_RADIUS - 0.001,
                "bots overlapped before the one-second recovery threshold"
            );
        }
    }
    let world = app.world();
    assert!(
        position(world, worker).y > hauler_start.y && position(world, hauler).y < worker_start.y,
        "loaded bots failed to pass: {:?}, {:?}",
        position(world, worker),
        position(world, hauler)
    );
    for bot in [worker, hauler] {
        assert_eq!(world.get::<Cargo>(bot).unwrap().amount, 4);
    }
}
