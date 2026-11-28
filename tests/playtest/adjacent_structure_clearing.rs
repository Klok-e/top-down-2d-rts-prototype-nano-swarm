//! Adjacent completing sites share clearance without crossing physical blockers.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{Commitment, PlannedStructure, StructureClearing},
    navigation::Obstacle,
    resources::Stockpile,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn adjacent_structure_clearing_uses_free_exit_beside_congested_site() {
    let mut app = common::sim_app_with_planned();
    let upper = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let lower = common::spawn_planned_structure_at_cell(&mut app, IVec2::Y);
    for (plan, center, builder) in [
        (upper, Vec2::new(252., 324.), Vec2::new(180., 324.)),
        (lower, Vec2::new(252., 252.), Vec2::new(180., 252.)),
    ] {
        let state = *app.world().get::<PlannedStructure>(plan).unwrap();
        app.world_mut().entity_mut(plan).insert((
            Transform::from_translation(center.extend(0.)).with_scale(Vec3::new(1.125, 1.125, 1.)),
            state.with_work_remaining(0),
            StructureClearing::awaiting_validation(builder),
        ));
    }
    let occupant = common::spawn_defender_at(&mut app, Vec2::new(252., 324.));
    let congested = common::spawn_defender_at(&mut app, Vec2::new(252., 252.));
    let mut bodies = vec![occupant, congested];
    for position in [
        Vec2::new(180., 252.),
        Vec2::new(324., 252.),
        Vec2::new(252., 180.),
    ] {
        let stationary = common::spawn_worker_at(&mut app, position);
        app.world_mut()
            .entity_mut(stationary)
            .insert(Commitment::Working);
        bodies.push(stationary);
    }
    let blocker = common::spawn_structure_at(&mut app, Vec2::new(396., 324.));
    let blocker_shape = Obstacle::structure(app.world().get::<Transform>(blocker).unwrap());
    let mut previous: Vec<Vec2> = bodies
        .iter()
        .map(|body| {
            app.world()
                .get::<Transform>(*body)
                .unwrap()
                .translation
                .truncate()
        })
        .collect();
    for _ in 0..180 {
        app.update();
        let positions: Vec<Vec2> = bodies
            .iter()
            .map(|body| {
                app.world()
                    .get::<Transform>(*body)
                    .unwrap()
                    .translation
                    .truncate()
            })
            .collect();
        for (i, position) in positions.iter().enumerate() {
            assert!(
                previous[i].distance(*position) <= 5.001,
                "clearing must not teleport occupants"
            );
            assert!(
                blocker_shape.segment_clear(previous[i], *position),
                "local evacuation must not cross the adjacent blocker"
            );
        }
        previous = positions;
        if app.world().get::<Stockpile>(upper).is_some() {
            return;
        }
    }
    assert!(
        app.world().get::<Stockpile>(upper).is_some(),
        "the upper site must clear through its free exit while its neighbor is congested"
    );
}

#[test]
fn crowded_footprint_clears_through_a_friendly_at_the_sole_exit() {
    let mut app = common::sim_app_with_planned();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let center = Vec2::new(252., 252.);
    let exit = Vec2::new(324., 252.);
    let transform =
        Transform::from_translation(center.extend(0.)).with_scale(Vec3::new(1.125, 1.125, 1.));
    let state = *app.world().get::<PlannedStructure>(plan).unwrap();
    app.world_mut().entity_mut(plan).insert((
        transform,
        state.with_work_remaining(0),
        StructureClearing::awaiting_validation(exit),
    ));
    let walls = [
        (Vec2::new(180., 252.), 1.125),
        (Vec2::new(288., 180.), 4.5),
        (Vec2::new(288., 324.), 4.5),
    ]
    .map(|(position, width)| {
        let wall = common::spawn_structure_at(&mut app, position);
        *app.world_mut().get_mut::<Transform>(wall).unwrap() = transform
            .with_translation(position.extend(0.))
            .with_scale(Vec3::new(width, 1.125, 1.));
        Obstacle::structure(app.world().get::<Transform>(wall).unwrap())
    });
    let friendly = common::spawn_worker_at(&mut app, exit);
    app.world_mut()
        .entity_mut(friendly)
        .insert(Commitment::Working);
    let mut previous = [center, center + Vec2::X * 25.];
    let occupants = previous.map(|position| {
        let bot = common::spawn_worker_at(&mut app, position);
        app.world_mut().entity_mut(bot).insert(Commitment::Working);
        bot
    });
    let footprint = Obstacle::structure(&transform);
    let mut passed_friendly = [false; 2];
    let mut waited_for_last_occupant = false;
    for _ in 0..180 {
        app.update();
        let positions = occupants.map(|bot| {
            app.world()
                .get::<Transform>(bot)
                .unwrap()
                .translation
                .truncate()
        });
        assert_eq!(
            app.world()
                .get::<Transform>(friendly)
                .unwrap()
                .translation
                .truncate(),
            exit
        );
        for (index, (&start, &end)) in previous.iter().zip(&positions).enumerate() {
            assert!(
                start.distance(end) <= 5.001,
                "clearing displaced an occupant by more than one tick of travel"
            );
            assert!(
                walls.iter().all(|wall| wall.segment_clear(start, end)),
                "departure crossed a wall"
            );
            passed_friendly[index] |= end.distance(exit) < 20.;
        }
        let outside = positions
            .iter()
            .filter(|&&point| footprint.admits_body(point))
            .count();
        if outside < occupants.len() {
            assert!(
                app.world().get::<Stockpile>(plan).is_none(),
                "a body is still in the footprint"
            );
            waited_for_last_occupant |= outside == 1;
        }
        if app.world().get::<Stockpile>(plan).is_some() {
            assert!(waited_for_last_occupant);
            assert_eq!(passed_friendly, [true; 2]);
            return;
        }
        previous = positions;
    }
    panic!("occupants did not clear through the friendly Nanobot at the sole exit: {previous:?}");
}

#[test]
fn clearing_reselects_an_exit_blocked_by_a_hostile() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{ClearingEvacuation, SwarmId, SwarmMember},
        navigation::BODY_RADIUS,
    };

    let mut app = common::sim_app_with_planned();
    let plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::ZERO);
    let center = Vec2::new(252., 252.);
    let east_exit = center + Vec2::X * 72.;
    let west_exit = center - Vec2::X * 72.;
    let transform =
        Transform::from_translation(center.extend(0.)).with_scale(Vec3::new(1.125, 1.125, 1.));
    let state = *app.world().get::<PlannedStructure>(plan).unwrap();
    app.world_mut().entity_mut(plan).insert((
        transform,
        state.with_work_remaining(0),
        StructureClearing::awaiting_validation(west_exit),
    ));
    for y in [180., 324.] {
        let wall = common::spawn_structure_at(&mut app, Vec2::new(center.x, y));
        app.world_mut().get_mut::<Transform>(wall).unwrap().scale = Vec3::new(5.625, 1.125, 1.);
    }
    let start = center + Vec2::X * 8.;
    let occupant = common::spawn_worker_at(&mut app, start);
    app.world_mut()
        .entity_mut(occupant)
        .insert(Commitment::Working);

    for _ in 0..60 {
        app.update();
        if app.world().get::<ClearingEvacuation>(occupant).is_some() {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<ClearingEvacuation>(occupant)
            .map(|route| route.goal),
        Some(east_exit),
    );
    let blocker_position = east_exit + Vec2::X * 20.;
    let blocker = common::spawn_worker_at(&mut app, blocker_position);
    app.world_mut()
        .entity_mut(blocker)
        .insert((SwarmMember(SwarmId(7)), Commitment::Working));

    for _ in 0..90 {
        app.update();
        let current = app
            .world()
            .get::<Transform>(occupant)
            .unwrap()
            .translation
            .truncate();
        assert!(current.distance(blocker_position) >= 2. * BODY_RADIUS - 0.001);
        if app.world().get::<Stockpile>(plan).is_some() {
            assert!(current.x <= west_exit.x + 2., "exited at {current}");
            return;
        }
    }
    panic!(
        "a hostile occupying the first exit stranded the structure: {:?}",
        app.world().get::<ClearingEvacuation>(occupant)
    );
}
