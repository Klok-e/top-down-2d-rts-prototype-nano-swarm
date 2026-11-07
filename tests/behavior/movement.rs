use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::VelocityComponent;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn coincident_crowd_velocity_remains_finite() {
    let mut app = common::sim_app();
    let crowd = (0..8)
        .map(|_| common::spawn_worker_at(&mut app, Vec2::ZERO))
        .collect::<Vec<_>>();

    app.update();

    for entity in crowd {
        let world = app.world();
        let transform = world
            .entity(entity)
            .get::<Transform>()
            .expect("crowd transform");
        let velocity = world
            .entity(entity)
            .get::<VelocityComponent>()
            .expect("crowd velocity");
        assert!(transform.translation.is_finite());
        assert!(velocity.value.is_finite());
    }
}
