#[path = "../common/mod.rs"]
mod common;

use bevy::{prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    fixed_simulation_time,
    nanobot::{DirectMovementComponent, NanobotBundle},
};

fn moving_nanobot_app(frame_ticks: u32) -> App {
    let mut app = common::sim_app();
    app.insert_resource(fixed_simulation_time());
    app.insert_resource(TimeUpdateStrategy::FixedTimesteps(frame_ticks));
    // TimePlugin's first update establishes its baseline and intentionally has no
    // elapsed duration. Warm it before spawning simulation state.
    app.update();
    app.world_mut().spawn((
        NanobotBundle::default(),
        Transform::default(),
        DirectMovementComponent {
            xy: Vec2::new(10_000.0, 0.0),
            stop_radius: 0.0,
        },
    ));
    app
}

fn advance(mut app: App, updates: usize) -> Vec2 {
    for _ in 0..updates {
        app.update();
    }
    let world = app.world_mut();
    let mut query = world
        .query_filtered::<&Transform, With<top_down_2d_rts_prototype_nano_swarm::nanobot::Nanobot>>(
        );
    query
        .single(world)
        .expect("moving nanobot exists")
        .translation
        .truncate()
}

#[test]
fn equal_simulated_duration_is_independent_of_render_frame_partition() {
    let fine = advance(moving_nanobot_app(1), 60);
    let coarse = advance(moving_nanobot_app(6), 10);

    assert_eq!(fine, coarse);
    assert!(fine.x > 0.0, "the fixed simulation must advance movement");
}
