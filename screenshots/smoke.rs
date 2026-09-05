//! Sample screenshot test exercising the callback harness end to end.
//!
//! Drives full offscreen [`top_down_2d_rts_prototype_nano_swarm::build_app_with_presentation`]
//! stack: warm up 5 updates so default scenario (background, intent meshes,
//! camera, spawned swarms) renders, capture one screenshot, then exit.
//! Callback also asserts ECS state mid-run, showing deterministic assertions
//! and visual capture can coexist in same callback.
//!
//! Run: `cargo test --test screenshots -- --ignored smoke`
//! The artifact lands at `target/playtest-screenshots/smoke.png`
//! (gitignored with `/target`).

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::ProductionFacility,
    scenario::{PLAYER_CELL, cell_origin},
};

use crate::harness::{TestContext, TestFlow};

/// Smoke test: warm up, assert the camera exists, capture, exit.
pub fn smoke(ctx: &mut TestContext) -> TestFlow {
    // Startup must spawn a camera before deterministic updates reach this point.
    // This proves the harness drives full app startup, not an empty app.
    if ctx.frame == 2 {
        let mut camera = ctx.world.query::<&Camera>();
        assert!(
            camera.iter(ctx.world).count() >= 1,
            "default scenario should have spawned at least one camera by frame 2"
        );

        let facility_positions = ctx
            .world
            .query_filtered::<&Transform, With<ProductionFacility>>()
            .iter(ctx.world)
            .map(|transform| transform.translation.truncate())
            .collect::<Vec<_>>();
        // Authored 192-unit square snaps to x288..504, y144..360.
        let expected_facility = Vec2::new(396.0, 252.0);
        assert!(
            facility_positions
                .iter()
                .any(|pos| pos.abs_diff_eq(expected_facility, 0.001)),
            "default player Production Facility must spawn at its visible authored position"
        );
        assert_ne!(
            expected_facility,
            cell_origin(PLAYER_CELL),
            "default Production Facility must not be hidden under the seed swarm"
        );
    }

    // Warm up a few frames so the scene has rendered before capture.
    if ctx.frame < 5 {
        return TestFlow::Continue;
    }

    if ctx.frame == 5 {
        return TestFlow::Screenshot("smoke".to_string());
    }

    TestFlow::Exit
}
