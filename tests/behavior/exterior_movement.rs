#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{InteractionRegion, VelocityComponent};

#[test]
fn exterior_approach_stops_a_body_clear_of_a_scaled_structure_corner() {
    let mut app = common::sim_app();
    let target = Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::new(2.0, 1.0, 1.0));
    let start = Vec2::new(120.0, 90.0);
    let bot = app
        .world_mut()
        .spawn((
            Transform::from_translation(start.extend(0.0)),
            VelocityComponent::default(),
            InteractionRegion::structure(&target).movement_from(start),
        ))
        .id();
    for _ in 0..60 {
        app.update();
    }
    let pos = app
        .world()
        .get::<Transform>(bot)
        .unwrap()
        .translation
        .truncate();
    let corner_clearance = pos.distance(Vec2::new(64.0, 32.0));
    assert!(
        (34.0..=38.01).contains(&corner_clearance),
        "clearance={corner_clearance}, pos={pos}"
    );
    assert!(InteractionRegion::structure(&target).contains(pos));
    assert!(!InteractionRegion::structure(&target).contains(Vec2::ZERO));
    // Full rendered pixel extents require more than 32 units beside this edge.
    assert!(!InteractionRegion::structure(&target).contains(Vec2::new(96.0, 0.0)));
    assert!(!InteractionRegion::structure(&target).contains(Vec2::new(120.0, 0.0)));
}
