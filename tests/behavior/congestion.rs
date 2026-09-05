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
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Commitment, RemainingTravel, VelocityComponent, velocity_system,
    };
    let mut app = common::minimal_app();
    let step = Duration::from_millis(100);
    app.insert_resource(Time::<Fixed>::from_duration(step));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
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
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Cargo, Commitment, RemainingTravel, VelocityComponent, WaitingForWork, WorkBlocked,
        separation_system, velocity_system, work_standing_system,
    };
    use top_down_2d_rts_prototype_nano_swarm::resources::ResourceKind;
    let mut app = common::minimal_app();
    app.add_systems(
        FixedUpdate,
        (separation_system, velocity_system, work_standing_system).chain(),
    );
    for y in [252., 396.] {
        let wall = common::spawn_structure_at(&mut app, Vec2::new(432., y));
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(4.5, 1.125, 1.);
    }
    let worker = common::spawn_worker_at(&mut app, Vec2::new(432., 324.));
    app.world_mut()
        .entity_mut(worker)
        .insert(Commitment::Working);
    let loaded = common::spawn_hauler_at(&mut app, Vec2::new(360., 324.));
    app.world_mut().entity_mut(loaded).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 20,
        },
        DirectMovementComponent {
            xy: Vec2::new(504., 324.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        },
    ));
    let mut crossed = false;
    for _ in 0..40 {
        let x = app.world().get::<Transform>(loaded).unwrap().translation.x;
        app.world_mut().entity_mut(loaded).insert((
            RemainingTravel(504. - x),
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
        if position.distance(Vec2::new(432., 324.)) < 68. {
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
    for _ in 0..80 {
        app.update();
    }
    let position = app
        .world()
        .get::<Transform>(loaded)
        .unwrap()
        .translation
        .truncate();
    assert!(
        position.distance(Vec2::new(432., 324.)) >= 67.999,
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
            .distance(Vec2::new(432., 324.))
            < 0.001
    );
    assert_eq!(app.world().get::<Cargo>(loaded).unwrap().amount, 20);
}
