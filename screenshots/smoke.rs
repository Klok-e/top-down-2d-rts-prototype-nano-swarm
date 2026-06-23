//! Sample screenshot test exercising the callback harness end to end.
//!
//! Drives the full [`top_down_2d_rts_prototype_nano_swarm::build_app`]
//! stack: warm up 5 frames so the default scenario (background, intent
//! meshes, camera, spawned swarms) renders, capture one screenshot,
//! then exit. The callback also asserts on ECS state mid-run to show
//! that deterministic assertions and visual capture mix in the same
//! callback (`tests/` style assertions + a PNG).
//!
//! Run: `SCREENSHOT_TEST=1 cargo test --test screenshots -- smoke`
//! The artifact lands at `target/playtest-screenshots/smoke.png`
//! (gitignored with `/target`).

use bevy::prelude::*;

use crate::harness::{TestContext, TestFlow};

/// Smoke test: warm up, assert the camera exists, capture, exit.
pub fn smoke(ctx: &mut TestContext) -> TestFlow {
    // Assert on ECS state mid-run: the default scenario must have
    // spawned a camera. This proves the harness drives a real
    // `app.run()` with Startup executed, not an empty app.
    if ctx.frame == 2 {
        let mut camera = ctx.world.query::<&Camera>();
        assert!(
            camera.iter(ctx.world).count() >= 1,
            "default scenario should have spawned at least one camera by frame 2"
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
