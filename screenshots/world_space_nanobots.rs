//! Screenshot test for issue #38 / ADR-0004.
//!
//! Captures default scenario and asserts ECS world positions: worker must be
//! near deposit center, not cell corner. Artifact also shows production
//! structure with bots co-located.
//!
//! This is the visual half of the issue #38 acceptance
//! ("Screenshot evidence (via the `screenshots/` harness)
//! shows a worker at the deposit center and a structure
//! with bots co-located. The producing agent inspects the
//! image and states visual facts."). ECS checks below supplement visual
//! inspection; ignored harness performs offscreen GPU rendering and readback.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{ZONE_BLOCK_SIZE, nanobot::SwarmId};

use super::harness::{TestContext, TestFlow, run_screenshot_test};

const CELL_SIZE: f32 = ZONE_BLOCK_SIZE;

/// Gather deposit at cell `(-2, 0)` (= PLAYER_DEPOSIT_CELL).
const DEPOSIT_CELL: IVec2 = IVec2::new(-2, 0);

fn deposit_pos() -> Vec2 {
    Vec2::new(
        DEPOSIT_CELL.x as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        CELL_SIZE / 2.0,
    )
}

fn cell_corner_pos() -> Vec2 {
    deposit_pos() + Vec2::new(256.0, 256.0)
}

/// Capture after worker has had time to walk to deposit. Test is ignored by
/// default; run with `cargo test --test screenshots -- --ignored
/// world_space_nanobots`. Callback pauses until PNG readback completes, then
/// resumes and exits.
pub fn world_space_nanobots(ctx: &mut TestContext) -> TestFlow {
    // Full offscreen app startup initializes player/opponent swarms, default
    // Gather/Build/Defend paint, resource deposit at (-768, 256), production
    // facility, and ResourceLedger. Advance simulation until worker arrives.
    // Capture wait runs full app updates, so simulation may advance before the
    // callback resumes. No post-resume gameplay state is assumed.
    if ctx.frame < 250 {
        // Drive the simulation. The player swarm sits
        // at (256, 256); the deposit is at (-768, 256);
        // the worker must walk 1024 units. At 5
        // units/tick, that's ~205 ticks. 250 ticks
        // gives a small margin.
        return TestFlow::Continue;
    }
    if ctx.frame == 250 {
        // Assert ECS state before capture so simulation regressions fail with
        // position-specific diagnostics.
        let world = &mut *ctx.world;
        let deposit = deposit_pos();
        let corner = cell_corner_pos();
        // Find the player worker.
        let worker = world
            .query::<(
                Entity,
                &top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType,
                &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
            )>()
            .iter(world)
            .find(|(_, t, m)| {
                **t == top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker
                    && m.0 == SwarmId::PLAYER
            })
            .map(|(e, _, _)| e)
            .expect("a player Worker must exist in the default scenario");
        let bot_pos = world
            .entity(worker)
            .get::<Transform>()
            .expect("worker must have a Transform")
            .translation
            .truncate();
        let dist_to_deposit = bot_pos.distance(deposit);
        assert!(
            dist_to_deposit <= 200.0,
            "worker Transform should land within ~200 units of the deposit center ({:?}); got {:?}, distance = {}",
            deposit,
            bot_pos,
            dist_to_deposit
        );
        let dist_to_corner = bot_pos.distance(corner);
        assert!(
            dist_to_corner > 200.0,
            "worker must not land at the cell corner ({:?}); got {:?}, distance to corner = {}",
            corner,
            bot_pos,
            dist_to_corner
        );
        return TestFlow::Screenshot("world_space_nanobots".to_string());
    }
    TestFlow::Exit
}

/// Public entry point used by the harness. Mirrors the
/// pattern in `screenshots::smoke::smoke`.
#[allow(dead_code)]
pub fn run() -> Result<std::path::PathBuf, String> {
    run_screenshot_test(world_space_nanobots)
}
