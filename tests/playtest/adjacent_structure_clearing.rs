//! Adjacent completing sites cannot route their occupants into one another.
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
            for other in &positions[i + 1..] {
                assert!(
                    position.distance(*other) >= 67.999,
                    "clearing must preserve body separation"
                );
            }
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
