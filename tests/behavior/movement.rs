use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{DirectMovementComponent, VelocityComponent};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn combined_direct_and_separation_velocity_is_clamped_to_bot_speed() {
    let mut app = common::sim_app_with_movement();
    app.world_mut()
        .resource_mut::<top_down_2d_rts_prototype_nano_swarm::game_settings::GameSettings>()
        .bot_speed = 5.25;
    let mover = common::spawn_defender_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::X);
    app.world_mut()
        .entity_mut(mover)
        .insert(DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: Vec2::new(-100.0, 0.0),
            stop_radius: 0.0,
        });

    app.update();
    let before = app
        .world()
        .get::<Transform>(mover)
        .unwrap()
        .translation
        .truncate();
    app.update();

    let position = app
        .world()
        .entity(mover)
        .get::<Transform>()
        .expect("mover transform")
        .translation
        .truncate();
    assert!(
        (position - before).distance(Vec2::new(-5.25, 0.0)) <= 1e-4,
        "combined velocity must clamp to the configured 5.25 speed; displacement={position}",
    );
}

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
