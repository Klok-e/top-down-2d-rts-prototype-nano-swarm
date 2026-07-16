//! Screenshot test for issue #38 / ADR-0004.
//!
//! Captures default scenario and asserts ECS world positions: worker must be
//! near deposit center, not cell corner. Artifact also shows the seed Production
//! Facility visibly offset from the initial nanobot cluster.
//!
//! This is the visual half of the issue #38 acceptance
//! ("Screenshot evidence (via the `screenshots/` harness)
//! shows a worker at the deposit center and a structure
//! with bots co-located. The producing agent inspects the
//! image and states visual facts."). ECS checks below supplement visual
//! inspection; ignored harness performs offscreen GPU rendering and readback.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    nanobot::{OwnerSwarm, ProductionFacility, STRUCTURE_MAX_HEALTH, Structure, SwarmId},
    scenario::{PLAYER_CELL, SEED_FACILITY_OFFSET, cell_origin},
};

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
    if ctx.frame < 900 {
        // Drive the simulation. The player swarm sits at (256, 256); the
        // deposit is at (-768, 256); the worker must walk 1024 units. Running
        // 900 ticks also crosses the unmaintained facility-collapse horizon.
        return TestFlow::Continue;
    }
    if ctx.frame == 900 {
        // Assert ECS state before capture so simulation regressions fail with
        // position-specific diagnostics.
        let world = &mut *ctx.world;
        let deposit = deposit_pos();
        let corner = cell_corner_pos();
        let expected_facility = cell_origin(PLAYER_CELL) + SEED_FACILITY_OFFSET;
        let player_facility = world
            .query::<(&ProductionFacility, &OwnerSwarm, &Transform, &Structure)>()
            .iter(world)
            .find(|(_, owner, _, _)| {
                world
                    .entity(owner.0)
                    .get::<SwarmId>()
                    .is_some_and(|swarm| *swarm == SwarmId::PLAYER)
            })
            .map(|(_, _, transform, condition)| {
                (transform.translation.truncate(), condition.health)
            })
            .expect("default player Production Facility must survive 900 fixed ticks");
        assert_eq!(
            player_facility.0, expected_facility,
            "maintained seed facility must remain at its visible authored position"
        );
        assert_eq!(
            player_facility.1, STRUCTURE_MAX_HEALTH,
            "default Worker allocation must keep the seed facility fully maintained"
        );

        // Population growth may create several Workers. Assert against the
        // player Worker nearest the Gather target rather than entity order.
        let bot_pos = world
            .query::<(
                &top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType,
                &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
                &Transform,
            )>()
            .iter(world)
            .filter(|(kind, member, _)| {
                **kind == top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker
                    && member.0 == SwarmId::PLAYER
            })
            .map(|(_, _, transform)| transform.translation.truncate())
            .min_by(|left, right| {
                left.distance_squared(deposit)
                    .total_cmp(&right.distance_squared(deposit))
            })
            .expect("a player Worker must exist in the default scenario");
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
