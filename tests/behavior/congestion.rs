use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{
    Cargo, Commitment, DirectMovementComponent, InteractionRegion, WaitingForWork, WorkBlocked,
};
use top_down_2d_rts_prototype_nano_swarm::{navigation::BODY_RADIUS, resources::ResourceKind};
#[path = "../common/mod.rs"]
mod common;

fn position(app: &App, bot: Entity) -> Vec2 {
    app.world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate()
}

fn single_file_lane(app: &mut App, offset: Vec2) {
    for (center, scale) in [
        (Vec2::new(432., 252.), Vec3::new(4.5, 1.125, 1.)),
        (Vec2::new(432., 396.), Vec3::new(4.5, 1.125, 1.)),
        (Vec2::new(252., 324.), Vec3::new(1.125, 3.375, 1.)),
        (Vec2::new(612., 324.), Vec3::new(1.125, 3.375, 1.)),
    ] {
        let wall = common::spawn_structure_at(app, center + offset);
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = scale;
    }
}

fn travel_from(app: &mut App, bot: Entity, goal: Vec2) {
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: goal,
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
}

#[test]
fn a_loaded_hauler_passes_a_defender_on_its_way_to_a_charger_without_retreating() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Charger, ChargerAssignment};

    let mut app = common::sim_app_with_movement();
    single_file_lane(&mut app, Vec2::ZERO);
    let charger = common::spawn_structure_at(&mut app, Vec2::new(252., 324.));
    let transform = app
        .world_mut()
        .get_mut::<Transform>(charger)
        .unwrap()
        .with_scale(Vec3::new(1.125, 3.375, 1.));
    app.world_mut()
        .entity_mut(charger)
        .insert((transform, Charger::new(IVec2::ZERO)));
    let left = common::spawn_hauler_at(&mut app, Vec2::new(359., 324.));
    let right = common::spawn_defender_at(&mut app, Vec2::new(504., 324.));
    let destination = Vec2::new(504., 324.);
    let approach = InteractionRegion::structure(&transform);
    travel_from(&mut app, left, destination);
    app.world_mut().entity_mut(left).insert(Cargo {
        kind: ResourceKind::Minerals,
        amount: 7,
    });
    app.world_mut().entity_mut(right).insert((
        approach.movement_from(Vec2::new(504., 324.)),
        ChargerAssignment { charger },
    ));

    let mut crossed = false;
    let mut previous = [position(&app, left), position(&app, right)];
    for _ in 0..55 {
        app.update();
        let positions = [position(&app, left), position(&app, right)];
        crossed |= positions[0].distance(positions[1]) < 2. * BODY_RADIUS;
        assert!(
            positions[0].x >= previous[0].x && positions[1].x <= previous[1].x,
            "head-on travel retreated: {previous:?} -> {positions:?}"
        );
        for i in 0..2 {
            assert!((322.0..=326.0).contains(&positions[i].y));
            assert!(positions[i].distance(previous[i]) <= 5.001);
        }
        previous = positions;
    }
    assert!(crossed, "both trips must use the single lane");
    assert!(position(&app, left).distance(destination) < 3.);
    assert!(approach.contains(position(&app, right)));
    assert!(app.world().get::<DirectMovementComponent>(right).is_none());
    assert_eq!(app.world().get::<Cargo>(left).unwrap().amount, 7);
}

#[test]
fn a_hauler_passes_an_idle_friend_in_a_single_file_lane() {
    let mut app = common::sim_app_with_movement();
    single_file_lane(&mut app, Vec2::ZERO);
    let mover = common::spawn_hauler_at(&mut app, Vec2::new(360., 324.));
    let idle_position = Vec2::new(432., 324.);
    let idle = common::spawn_worker_at(&mut app, idle_position);
    let goal = Vec2::new(504., 324.);
    travel_from(&mut app, mover, goal);
    let mut crossed = false;
    for _ in 0..45 {
        app.update();
        crossed |= position(&app, mover).distance(idle_position) < 10.;
        assert_eq!(
            position(&app, idle),
            idle_position,
            "an idle friend is not displaced by ordinary travel"
        );
    }
    assert!(crossed);
    assert!(position(&app, mover).distance(goal) < 3.);
}

#[test]
fn a_loaded_hauler_waiting_after_overlap_spreads_away_and_keeps_its_cargo() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        RemainingTravel, VelocityComponent, separation_system, velocity_system,
        work_standing_system,
    };
    for offset in [Vec2::ZERO, Vec2::X * 4320.0] {
        let mut app = common::minimal_app();
        app.insert_resource(top_down_2d_rts_prototype_nano_swarm::intent::IntentGrid::new(32, 32));
        app.add_systems(
            FixedUpdate,
            (separation_system, velocity_system, work_standing_system).chain(),
        );
        single_file_lane(&mut app, offset);
        let worker_position = Vec2::new(432., 324.) + offset;
        let worker = common::spawn_worker_at(&mut app, worker_position);
        app.world_mut()
            .entity_mut(worker)
            .insert(Commitment::Working);
        let loaded = common::spawn_hauler_at(&mut app, Vec2::new(360., 324.) + offset);
        app.world_mut().entity_mut(loaded).insert(Cargo {
            kind: ResourceKind::Minerals,
            amount: 20,
        });
        travel_from(&mut app, loaded, Vec2::new(504., 324.) + offset);

        for _ in 0..15 {
            app.world_mut()
                .entity_mut(loaded)
                .insert(VelocityComponent {
                    value: Vec2::X * 5.,
                });
            app.update();
            assert!(
                app.world().get::<WorkBlocked>(worker).is_none(),
                "friendly passage must leave the work position admitted"
            );
        }
        assert!(
            position(&app, loaded).distance(worker_position) < 2. * BODY_RADIUS - 10.,
            "fixture must enter transit overlap at {offset:?}, travelled to {:?}",
            position(&app, loaded)
        );
        // Destination capacity disappears while the bots overlap.
        app.world_mut()
            .entity_mut(loaded)
            .insert(WaitingForWork)
            .remove::<RemainingTravel>();
        for _ in 0..240 {
            app.update();
        }
        assert!(position(&app, loaded).distance(worker_position) >= 2. * BODY_RADIUS - 0.002);
        assert!(app.world().get::<WorkBlocked>(worker).is_none());
        assert_eq!(position(&app, worker), worker_position);
        assert_eq!(app.world().get::<Cargo>(loaded).unwrap().amount, 20);
    }
}

#[test]
fn idle_friends_spread_after_arriving_at_the_same_destination() {
    let mut app = common::sim_app_with_movement();
    let goal = Vec2::new(400., 300.);
    let bots = [
        common::spawn_worker_at(&mut app, goal - Vec2::X * 130.),
        common::spawn_hauler_at(&mut app, goal + Vec2::X * 130.),
    ];
    for bot in bots {
        travel_from(&mut app, bot, goal);
    }
    let mut shared_destination = false;
    for _ in 0..240 {
        app.update();
        shared_destination |= bots
            .into_iter()
            .all(|bot| position(&app, bot).distance(goal) < 3.);
    }
    assert!(shared_destination);
    assert!(position(&app, bots[0]).distance(position(&app, bots[1])) >= 2. * BODY_RADIUS - 0.002);
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
    assert!(closest_spacing < 2.0 * BODY_RADIUS);
    assert_eq!(app.world().get::<Cargo>(traveller).unwrap().amount, 4);
}

#[test]
fn loaded_head_on_trips_pass_through_a_bend_with_both_detours_blocked() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, RemainingTravel, VelocityComponent, velocity_system},
        navigation::Navigation,
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
