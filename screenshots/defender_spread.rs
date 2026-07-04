//! Screenshot test for issue #37: defender spatial pressure and
//! in-cell holding.
//!
//! Captures a frame after a handful of player defenders have had
//! time to spread across a small plus of strongly-painted Defend
//! cells near the world origin. The deterministic ECS assertion
//! (defenders hold DISTINCT cells, each INSIDE its assigned cell)
//! is the primary evidence; the screenshot is the visual
//! confirmation that the spread is real and not a cluster on one
//! cell center.
//!
//! Run: `cargo test --test screenshots -- --ignored defender_spread`
//! The artifact lands at
//! `target/playtest-screenshots/defender_spread.png` (gitignored
//! with `/target`).

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        point_in_cell, Charge, Commitment, DefendHold, Health, Nanobot, NanobotType, SwarmId,
        SwarmMember, VelocityComponent,
    },
};

use crate::harness::{run_screenshot_test, TestContext, TestFlow};

/// The plus of Defend cells painted around the world origin.
/// Centers (0,0), (1,0), (-1,0), (0,1), (0,-1) -- five cells the
/// defenders can spread across. Chosen so the camera (centered on
/// the origin, orthographic scale 2.0) frames the whole plus.
const DEFEND_CELLS: [IVec2; 5] = [
    IVec2::new(0, 0),
    IVec2::new(1, 0),
    IVec2::new(-1, 0),
    IVec2::new(0, 1),
    IVec2::new(0, -1),
];

/// Number of defenders the test spawns. Fewer than cells so the
/// spread is unambiguous (each defender can hold a distinct cell).
const DEFENDER_COUNT: usize = 4;

fn origin_center() -> Vec2 {
    Vec2::new(256.0, 256.0)
}

/// Paint the plus of Defend cells and spawn the defenders on the
/// first frame, then let the simulation spread them. After enough
/// frames for travel + arrival, capture and assert.
pub fn defender_spread(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        let world = &mut *ctx.world;
        // Paint the Defend plus, stamped with player ownership so
        // the per-swarm intent filter routes player defenders to
        // these cells.
        {
            let mut grid = world.resource_mut::<IntentGrid>();
            for cell in DEFEND_CELLS {
                grid.add_owned(
                    cell,
                    IntentKind::Defend,
                    PAINT_STRENGTH_CAP,
                    Some(SwarmId::PLAYER),
                );
            }
        }
        // Spawn the defenders at the (0,0) cell center.
        let spawn = origin_center();
        for _ in 0..DEFENDER_COUNT {
            world.spawn((
                Nanobot {},
                NanobotType::Defender,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                Charge::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_translation(spawn.extend(0.0)),
            ));
        }
        return TestFlow::Continue;
    }

    // Drive the simulation. Defenders walk to their assigned
    // cells (max ~1 cell = 512 units at 5 units/tick; the in-cell
    // stop radius shortens the walk to ~60 ticks). 90 frames is
    // a safe margin for travel + arrival + hold stabilization,
    // and early enough that the charge sustain loop has not yet
    // drained the defenders (no charger is built this fast, so
    // waiting longer would let empty-charge health loss despawn
    // them -- see `src/nanobot/charge.rs`).
    if ctx.frame < 90 {
        return TestFlow::Continue;
    }

    if ctx.frame == 90 {
        // Deterministic assertion: the defenders must hold at
        // least three DISTINCT cells (proving spread, not a
        // cluster), and every holder must be physically INSIDE
        // its held cell (proving in-cell holding, not center
        // clustering or drift out of cell).
        let world = &mut *ctx.world;
        let mut holds: Vec<(IVec2, Vec2)> = Vec::new();
        for (hold, transform, member, ntype) in world
            .query::<(&DefendHold, &Transform, &SwarmMember, &NanobotType)>()
            .iter(world)
        {
            // Only count player defenders -- the default opponent
            // scenario also has a defender holding its own cell.
            if *ntype != NanobotType::Defender || member.0 != SwarmId::PLAYER {
                continue;
            }
            holds.push((hold.cell, transform.translation.truncate()));
        }
        assert!(
            !holds.is_empty(),
            "player defenders should have arrived and be holding by frame 90"
        );
        let distinct_cells: std::collections::HashSet<IVec2> =
            holds.iter().map(|(c, _)| *c).collect();
        assert!(
            distinct_cells.len() >= 3,
            "defenders must spread across at least 3 distinct cells; got {} ({:?})",
            distinct_cells.len(),
            distinct_cells
        );
        for (cell, pos) in &holds {
            assert!(
                point_in_cell(*pos, *cell),
                "holding defender must be inside its assigned cell {cell}; pos={pos}"
            );
        }
        return TestFlow::Screenshot("defender_spread".to_string());
    }

    TestFlow::Exit
}

/// Public entry point used by the harness. Mirrors the pattern in
/// `screenshots::smoke::smoke`.
#[allow(dead_code)]
pub fn run() -> Result<std::path::PathBuf, String> {
    run_screenshot_test(defender_spread)
}
