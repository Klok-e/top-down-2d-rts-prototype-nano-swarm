//! Screenshot test for issue #37: defender spatial pressure and
//! in-cell holding.
//!
//! Captures player defenders spreading across isolated plus of strongly-painted
//! Defend cells. Deterministic ECS assertion (defenders hold distinct cells,
//! each inside its assigned cell)
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
    intent::{IntentGrid, IntentKind},
    nanobot::{
        point_in_cell, Charge, Commitment, DefendHold, Health, Nanobot, NanobotSprites,
        NanobotType, SwarmId, SwarmMember, VelocityComponent,
    },
    ZONE_BLOCK_SIZE,
};

use crate::harness::{run_screenshot_test, TestContext, TestFlow};

#[derive(Component)]
struct SpreadTestDefender;

/// Plus lives away from default scenario paint, keeping visual evidence isolated.
/// Default orthographic scale 2.0 frames all five cells.
const CENTER_CELL: IVec2 = IVec2::new(0, 5);
const DEFEND_CELLS: [IVec2; 5] = [
    CENTER_CELL,
    IVec2::new(1, 5),
    IVec2::new(-1, 5),
    IVec2::new(0, 6),
    IVec2::new(0, 4),
];

/// Number of defenders the test spawns. Fewer than cells so the
/// spread is unambiguous (each defender can hold a distinct cell).
const DEFENDER_COUNT: usize = 4;

fn defend_center() -> Vec2 {
    Vec2::new(
        (CENTER_CELL.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (CENTER_CELL.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

fn prepare_view(world: &mut World) {
    let target = defend_center();
    for mut transform in world
        .query_filtered::<&mut Transform, With<Camera2d>>()
        .iter_mut(world)
    {
        transform.translation.x = target.x;
        transform.translation.y = target.y;
    }
}

/// Paint the plus of Defend cells and spawn the defenders on the
/// first frame, then let the simulation spread them. After enough
/// frames for travel + arrival, capture and assert.
pub fn defender_spread(ctx: &mut TestContext) -> TestFlow {
    let world = &mut *ctx.world;
    if ctx.frame == 0 {
        prepare_view(world);
        // Paint the Defend plus, stamped with player ownership so
        // the per-swarm intent filter routes player defenders to
        // these cells.
        {
            let mut grid = world.resource_mut::<IntentGrid>();
            for cell in DEFEND_CELLS {
                grid.add_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER));
            }
        }
        // Spawn visible defenders at isolated center cell.
        let spawn = defend_center();
        let sprite = world
            .resource::<NanobotSprites>()
            .handle(NanobotType::Defender, false);
        for _ in 0..DEFENDER_COUNT {
            world.spawn((
                SpreadTestDefender,
                Nanobot {},
                NanobotType::Defender,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                Charge::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_translation(spawn.extend(0.0)),
                Sprite::from_image(sprite.clone()),
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
        let mut holds: Vec<(IVec2, Vec2)> = Vec::new();
        for (hold, transform, member, ntype) in world
            .query_filtered::<
                (&DefendHold, &Transform, &SwarmMember, &NanobotType),
                With<SpreadTestDefender>,
            >()
            .iter(world)
        {
            // Count only marked player defenders spawned by this test.
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
