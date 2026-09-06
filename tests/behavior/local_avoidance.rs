use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent;
#[path = "../common/mod.rs"]
mod common;

#[test]
fn opposing_types_pass_structure_corners_and_deposits_with_body_clearance() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{NanobotType, SwarmId, SwarmMember};
    for kind in [
        NanobotType::Worker,
        NanobotType::Hauler,
        NanobotType::Defender,
    ] {
        let mut app = common::sim_app_with_movement();
        let wall = common::spawn_structure_at(&mut app, Vec2::new(432., 324.));
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(4.5, 2.25, 1.);
        common::spawn_deposit(
            &mut app,
            common::DepositFixture {
                world_pos: Vec2::new(720., 324.),
                amount: 0,
                capacity: 20,
                radius: 72.,
            },
        );
        let starts = [Vec2::new(180., 324.), Vec2::new(1008., 324.)];
        let goals = [starts[1], starts[0]];
        let bots = starts.map(|position| common::spawn_worker_at(&mut app, position));
        for ((entity, xy), swarm) in bots
            .into_iter()
            .zip(goals)
            .zip([SwarmId::PLAYER, SwarmId(42)])
        {
            app.world_mut().entity_mut(entity).insert((
                kind,
                SwarmMember(swarm),
                DirectMovementComponent {
                    xy,
                    stop_radius: 0.,
                    interaction: None,
                    speed: None,
                },
            ));
        }
        let mut previous = starts;
        let mut completed_routes = 0;
        for tick in 0..1000 {
            app.update();
            completed_routes += app
                .world()
                .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
                .work()
                .completed;
            let now = bots.map(|entity| {
                app.world()
                    .get::<Transform>(entity)
                    .unwrap()
                    .translation
                    .truncate()
            });
            // Substeps check the literal authored shapes independently of navigation predicates.
            for sample in 0..=20 {
                let points =
                    [0, 1].map(|index| previous[index].lerp(now[index], sample as f32 / 20.));
                assert!(
                    points[0].distance(points[1]) >= 67.99,
                    "{kind:?} pair overlap at tick {tick}: {previous:?} -> {now:?}"
                );
                for point in points {
                    let wall_distance = ((point - Vec2::new(432., 324.)).abs()
                        - Vec2::new(144., 72.))
                    .max(Vec2::ZERO)
                    .length();
                    assert!(
                        wall_distance >= 33.99,
                        "{kind:?} clipped corner at {point:?}"
                    );
                    assert!(
                        point.distance(Vec2::new(720., 324.)) >= 105.99,
                        "{kind:?} clipped depleted deposit at {point:?}"
                    );
                }
            }
            previous = now;
        }
        assert_eq!(
            completed_routes, 2,
            "crowds must not request a global detour"
        );
        for index in 0..2 {
            assert!(
                previous[index].distance(goals[index]) < 3.,
                "{kind:?} stalled: {previous:?}"
            );
        }
    }
}

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
fn trapped_hostile_traffic_waits_without_overlap_or_wall_crossing() {
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
    app.world_mut().entity_mut(b).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember(
            top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId(42),
        ),
    );
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
        let mut reached = [false; 4];
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
                reached[first] |= now[first].distance(goals[first]) < 3.;
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
                        (relative + travel * t).length() >= 67.99
                            || [bots[first], bots[second]].into_iter().any(|bot| app.world().get::<top_down_2d_rts_prototype_nano_swarm::nanobot::CongestionRecovery>(bot).is_some()),
                        "stream collision: {previous:?} -> {now:?}"
                    );
                }
            }
            previous = now;
        }
        assert!(
            reached.into_iter().all(|arrived| arrived),
            "stream never reached its destinations: {previous:?}"
        );
        for bot in bots {
            assert!(
                app.world().get::<DirectMovementComponent>(bot).is_none(),
                "traffic still has unfinished travel"
            );
        }
    }
}

#[test]
fn queued_movement_waits_safely_and_retries_after_an_obstacle_is_removed() {
    use top_down_2d_rts_prototype_nano_swarm::navigation_runtime::NavigationBudget;
    let mut app = common::sim_app_with_movement();
    app.world_mut().resource_mut::<NavigationBudget>().0 = 0;
    let initial_wall = common::spawn_structure_at(&mut app, Vec2::new(400., 300.));
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
    app.world_mut().despawn(initial_wall);
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
    for _ in 0..100 {
        app.update();
        let x = app.world().get::<Transform>(bot).unwrap().translation.x;
        assert!(
            x <= 334.001,
            "pending replacement entered the structure: x={x}"
        );
    }
    let waiting = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    assert!(
        waiting.x > before.x + 20.0,
        "pending routing must preserve forward progress along the valid prefix"
    );
    assert!(
        waiting.x <= 334.001,
        "pending replacement entered the blocking structure: {waiting:?}"
    );
    assert!((waiting.y - 300.).abs() < 0.001);
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

#[test]
fn following_bot_slows_smoothly_at_occupied_space() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Commitment, VelocityComponent, velocity_system,
    };
    for gap in [71.0, 69.5] {
        let mut app = common::minimal_app();
        app.add_systems(Update, velocity_system);
        let follower = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
        let working = common::spawn_worker_at(&mut app, Vec2::new(200. + gap, 300.));
        app.world_mut()
            .entity_mut(working)
            .insert(Commitment::Working);
        app.world_mut().entity_mut(follower).insert((
            DirectMovementComponent {
                xy: Vec2::new(600., 300.),
                stop_radius: 0.,
                interaction: None,
                speed: None,
            },
            VelocityComponent {
                value: Vec2::new(5., 0.),
            },
        ));
        app.update();
        let position = app.world().get::<Transform>(follower).unwrap().translation;
        let forward = position.x - 200.;
        let lateral = (position.y - 300.).abs();
        assert!(
            forward > 0.0 && forward < 5.0,
            "follower did not slow: {position:?}"
        );
        assert!(
            lateral <= forward + 0.001,
            "sharp avoidable sidestep: {position:?}"
        );
        assert!(position.truncate().distance(Vec2::new(200. + gap, 300.)) >= 67.999);
        let work_position = app.world().get::<Transform>(working).unwrap().translation;
        assert!((work_position.x - (200. + gap)).abs() < 0.001);
        assert!((work_position.y - 300.).abs() < 0.001);
    }
}

#[test]
fn idle_work_approach_occupant_departs_for_distant_friendly_demand() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        ApproachPhase, Commitment, InteractionRegion, SwarmId, SwarmMember, WorkApproach,
        velocity_system,
    };
    for (swarm, commitment, should_depart) in [
        (SwarmId::PLAYER, Commitment::Idle, true),
        (SwarmId(42), Commitment::Idle, false),
        (SwarmId::PLAYER, Commitment::Working, false),
    ] {
        let mut app = common::minimal_app();
        app.add_systems(Update, velocity_system);
        let region = InteractionRegion::deposit(&Transform::from_xyz(400., 300., 0.), 36.);
        let occupant = common::spawn_worker_at(&mut app, Vec2::new(328., 300.));
        app.world_mut().entity_mut(occupant).insert(commitment);
        let carrier = common::spawn_worker_at(&mut app, Vec2::new(100., 300.));
        app.world_mut().entity_mut(carrier).insert((
            SwarmMember(swarm),
            WorkApproach {
                region,
                phase: ApproachPhase::Travelling,
                since: 0.,
            },
        ));
        for _ in 0..20 {
            app.update();
        }
        let position = app
            .world()
            .get::<Transform>(occupant)
            .unwrap()
            .translation
            .truncate();
        if should_depart {
            assert!(
                position.distance(region.approach(position)) >= 67.99,
                "idle occupant did not clear working space: {position:?}"
            );
        } else {
            assert!(
                position.distance(Vec2::new(328., 300.)) < 0.001,
                "undemanded or active work displaced: {position:?}"
            );
        }
    }
}

#[test]
fn newly_arrived_cargo_and_loading_assignments_stay_put_until_work_effects_run() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            ApproachPhase, Cargo, HaulerAssignment, InteractionRegion, PlannedStructureClaim,
            WorkApproach, velocity_system,
        },
        resources::ResourceKind,
    };
    for assignment in 0..3 {
        let mut app = common::minimal_app();
        app.add_systems(Update, velocity_system);
        let region = InteractionRegion::deposit(&Transform::from_xyz(400., 300., 0.), 36.);
        let arrival = common::spawn_worker_at(&mut app, Vec2::new(328., 300.));
        let demand = common::spawn_worker_at(&mut app, Vec2::new(100., 300.));
        app.world_mut().entity_mut(demand).insert(WorkApproach {
            region,
            phase: ApproachPhase::Travelling,
            since: 0.,
        });
        if assignment == 0 {
            app.world_mut().entity_mut(arrival).insert(Cargo {
                kind: ResourceKind::Minerals,
                amount: 4,
            });
        } else if assignment == 1 {
            app.world_mut()
                .entity_mut(arrival)
                .insert(HaulerAssignment {
                    source: demand,
                    sink: demand,
                });
        } else {
            app.world_mut()
                .entity_mut(arrival)
                .insert(PlannedStructureClaim {
                    target: demand,
                    cell: IVec2::ZERO,
                });
        }
        app.update();
        let position = app
            .world()
            .get::<Transform>(arrival)
            .unwrap()
            .translation
            .truncate();
        assert!(
            position.distance(Vec2::new(328., 300.)) < 0.001,
            "arrival displaced before work could begin (assignment={assignment}): {position:?}"
        );
        assert!(region.contains(position));
    }
}

#[test]
fn normal_steering_accelerates_and_turns_without_velocity_snaps() {
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{VelocityComponent, velocity_system};
    let mut app = common::minimal_app();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
    app.add_systems(FixedUpdate, velocity_system);
    let bot = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(900., 900.),
            stop_radius: 0.,
            speed: None,
            interaction: None,
        });
    let mut previous = Vec2::new(200., 300.);
    let mut previous_velocity = Vec2::ZERO;
    for tick in 0..24 {
        let requested = if tick < 12 {
            Vec2::new(5., 0.)
        } else {
            Vec2::new(0., 5.)
        };
        app.world_mut()
            .get_mut::<VelocityComponent>(bot)
            .unwrap()
            .value = requested;
        app.update();
        let position = app
            .world()
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate();
        let velocity = position - previous;
        assert!(
            velocity.distance(previous_velocity) <= 0.71,
            "normal movement snapped at tick {tick}: {previous_velocity:?} -> {velocity:?}"
        );
        if tick == 11 {
            assert!(
                (velocity.x - 5.).abs() < 0.01,
                "start is unresponsive: {velocity:?}"
            );
        }
        if tick == 23 {
            assert!(
                (velocity.y - 5.).abs() < 0.01,
                "turn is unresponsive: {velocity:?}"
            );
        }
        previous = position;
        previous_velocity = velocity;
    }
}

#[test]
fn arrival_brakes_progressively_without_overshooting_the_destination() {
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        RemainingTravel, VelocityComponent, velocity_system,
    };
    let mut app = common::minimal_app();
    let cadence = Duration::from_secs_f64(1.0 / 60.0);
    app.insert_resource(Time::<Fixed>::from_duration(cadence));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(cadence));
    app.add_systems(FixedUpdate, velocity_system);
    let bot = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(260., 300.),
            stop_radius: 0.,
            speed: None,
            interaction: None,
        });
    let mut previous_x = 200.;
    let mut previous_speed = 0.0_f32;
    for tick in 0..60 {
        app.world_mut().entity_mut(bot).insert((
            RemainingTravel(260. - previous_x),
            VelocityComponent {
                value: Vec2::new(5., 0.),
            },
        ));
        app.update();
        let x = app.world().get::<Transform>(bot).unwrap().translation.x;
        let speed = x - previous_x;
        assert!(
            (speed - previous_speed).abs() <= 0.71,
            "arrival snapped at tick {tick}: {previous_speed} -> {speed}"
        );
        assert!(
            (previous_x - 0.001..=260.001).contains(&x),
            "overshot or reversed at tick {tick}: {x}"
        );
        previous_speed = speed;
        previous_x = x;
    }
    assert!(
        (previous_x - 260.).abs() < 0.01,
        "failed to settle at destination: {previous_x}"
    );
}

#[test]
fn traveller_passes_a_stationary_hostile_without_displacing_it() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Commitment, SwarmId, SwarmMember};
    let mut app = common::sim_app_with_movement();
    let traveller = common::spawn_worker_at(&mut app, Vec2::new(200., 300.));
    let hostile = common::spawn_worker_at(&mut app, Vec2::new(300., 300.));
    app.world_mut()
        .entity_mut(hostile)
        .insert((SwarmMember(SwarmId(42)), Commitment::Working));
    app.world_mut()
        .entity_mut(traveller)
        .insert(DirectMovementComponent {
            xy: Vec2::new(600., 300.),
            stop_radius: 0.,
            speed: None,
            interaction: None,
        });
    let mut previous = Vec2::new(200., 300.);
    for tick in 0..300 {
        app.update();
        let position = app
            .world()
            .get::<Transform>(traveller)
            .unwrap()
            .translation
            .truncate();
        let delta = position - previous;
        let closest_t = if delta.length_squared() > 0.0 {
            ((Vec2::new(300., 300.) - previous).dot(delta) / delta.length_squared()).clamp(0., 1.)
        } else {
            0.0
        };
        assert!(
            (previous + delta * closest_t).distance(Vec2::new(300., 300.)) >= 67.999,
            "crossed hostile at tick {tick}: {previous:?} -> {position:?}"
        );
        assert!(
            app.world()
                .get::<Transform>(hostile)
                .unwrap()
                .translation
                .truncate()
                .distance(Vec2::new(300., 300.))
                < 0.001
        );
        previous = position;
    }
    assert!(
        previous.distance(Vec2::new(600., 300.)) < 3.,
        "stalled at stationary hostile: {previous:?}"
    );
}
