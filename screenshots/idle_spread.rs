//! Screenshot test for issue #39: idle cosmetic spread.
//!
//! Captures a frame showing idle haulers mid-drift toward a painted
//! Corridor segment, proving the spread system produces visible
//! movement the player can see (the "cosmetic liveness" goal). The
//! deterministic ECS assertion (the stranded idle haulers have moved
//! east toward the corridor, all still `Commitment::Idle` with no
//! `DirectMovementComponent`) is the primary evidence; the screenshot
//! is the visual confirmation that the drift is real.
//!
//! ## What the screenshot shows
//!
//! The in-region gradient path (a stacked cluster on painted cells)
//! is a gentle random walk -- per-tick re-evaluation with the random
//! tie-break produces sub-cell "quivering" liveness rather than rapid
//! cross-cell redistribution, which a single static frame cannot
//! capture. The stranded-seek path, by contrast, is deterministic:
//! an idle bot on an unpainted cell drifts in a straight line toward
//! the nearest fit-paint cell. That straight-line drift is the
//! clearest visible evidence of the spread system in a static frame,
//! so the screenshot exercises it: a small cluster of idle haulers
//! starts stranded just west of a Corridor segment and is captured
//! mid-drift heading east into it.
//!
//! ## Keeping the haulers idle + the frame clean
//!
//! Idle bots are transient: the haul assignment system grabs them the
//! moment a matching logistics leg exists. The test spawns the
//! haulers under a fresh [`SwarmId`] that owns no stockpiles,
//! facilities, or chargers; the haul leg picker's
//! `owner_matches_hauler` rule rejects legs whose source/sink owner
//! does not match the hauler's swarm, and the scenario's stockpiles
//! are all owner-stamped, so the haulers are never grabbed. The
//! central demand allocator ignores non-workers and the defend system
//! ignores non-defenders, so the haulers stay `Commitment::Idle`
//! indefinitely and the spread system is the only thing moving them.
//!
//! The default scenario paints a busy cluster of zones around the
//! world origin, so the callback relocates the camera to an empty
//! region far south of the scenario and paints the Corridor segment
//! and spawn point there. The captured frame shows only the spread
//! test setup, not the scenario clutter.
//!
//! Run: `cargo test --test screenshots -- --ignored idle_spread`
//! The artifact lands at `target/playtest-screenshots/idle_spread.png`
//! (gitignored with `/target`).

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        world_to_cell, Commitment, DirectMovementComponent, Health, Nanobot, NanobotType, SwarmId,
        SwarmMember, VelocityComponent,
    },
    ZONE_BLOCK_SIZE,
};

use crate::harness::{run_screenshot_test, TestContext, TestFlow};

/// Marker component tagging the haulers under test.
#[derive(Debug, Component)]
struct SpreadTestBot;

/// A swarm id that owns nothing in the default scenario. The haul
/// leg picker only matches legs whose source/sink owner is `None`
/// (unowned) or equals the hauler's swarm; the scenario stamps every
/// stockpile / facility / charger with a real owner, so this id has
/// no matching leg and its haulers are never grabbed.
const IDLE_SWARM: SwarmId = SwarmId(99);

/// Row far south of the scenario (player at y~0, opponent at y~0)
/// where the test paints its isolated Corridor segment.
const CORRIDOR_ROW: i32 = -5;

/// Corridor cells painted as a short horizontal segment east of the
/// spawn point. Each stranded hauler's nearest fit-paint cell is the
/// western end of this segment, so the drift is purely eastward and
/// deterministic.
const CORRIDOR_CELLS: [IVec2; 2] = [IVec2::new(1, CORRIDOR_ROW), IVec2::new(2, CORRIDOR_ROW)];

/// Spawn cell for the cluster of idle haulers, west of the corridor
/// on the same row.
const START_CELL: IVec2 = IVec2::new(-2, CORRIDOR_ROW);

/// Number of idle haulers stacked at the spawn cell.
const HAULER_COUNT: usize = 3;

/// Camera look-at point: the midpoint between spawn and corridor, on
/// the corridor row. Frames the whole drift in the default
/// orthographic scale (2.0), which the `defender_spread` test shows
/// frames ~5 cells.
fn camera_target() -> Vec2 {
    cell_center(IVec2::new(0, CORRIDOR_ROW))
}

fn cell_center(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

/// Pin the camera to the isolated test region every frame. The fly
/// camera integrates velocity, but with no keyboard input its
/// velocity stays zero, so setting the translation directly is not
/// fought by the movement system.
fn pin_camera(world: &mut World) {
    let target = camera_target();
    for mut transform in world
        .query_filtered::<&mut Transform, With<Camera2d>>()
        .iter_mut(world)
    {
        transform.translation.x = target.x;
        transform.translation.y = target.y;
    }
}

pub fn idle_spread(ctx: &mut TestContext) -> TestFlow {
    let world = &mut *ctx.world;
    pin_camera(world);

    if ctx.frame == 0 {
        // Paint the Corridor segment unowned so it is visible to
        // every swarm (including the idle test swarm).
        {
            let mut grid = world.resource_mut::<IntentGrid>();
            for &cell in &CORRIDOR_CELLS {
                grid.add_owned(cell, IntentKind::Corridor, PAINT_STRENGTH_CAP, None);
            }
        }
        // Spawn the idle haulers stacked at the stranded start cell.
        let spawn = cell_center(START_CELL);
        let hauler_sprite = world
            .get_resource::<top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotSprites>()
            .map(|s| Sprite::from_image(s.handle(NanobotType::Hauler, false)));
        for _ in 0..HAULER_COUNT {
            let mut entity = world.spawn((
                SpreadTestBot,
                Nanobot {},
                NanobotType::Hauler,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(IDLE_SWARM),
                Transform::from_translation(spawn.extend(0.0)),
            ));
            if let Some(sprite) = hauler_sprite.clone() {
                entity.insert(sprite);
            }
        }
        return TestFlow::Continue;
    }

    // Spread force is ~1.5 world units / tick. The haulers must cover
    // ~500 units to advance one cell east of their start; ~500 frames
    // puts them clearly mid-drift toward the corridor -- the clearest
    // static-frame evidence of movement.
    if ctx.frame < 500 {
        return TestFlow::Continue;
    }

    if ctx.frame == 500 {
        let mut moved_east = 0usize;
        for (transform, commitment, dmc) in world
            .query_filtered::<(&Transform, &Commitment, Option<&DirectMovementComponent>), With<SpreadTestBot>>(
            )
            .iter(world)
        {
            let pos = transform.translation.truncate();
            let cell = world_to_cell(pos);
            // Start cell.x = -2. After drifting east toward the
            // corridor the hauler must have advanced at least one
            // cell (cell.x >= -1) and moved east in world space.
            assert!(
                cell.x >= -1,
                "stranded hauler must drift east toward the corridor; start cell.x=-2 now at {cell} (pos {pos})"
            );
            assert!(
                pos.x > cell_center(START_CELL).x,
                "hauler must move east in world space; got {pos}"
            );
            assert_eq!(*commitment, Commitment::Idle, "spread must keep haulers idle");
            assert!(
                dmc.is_none(),
                "spread must not insert a DirectMovementComponent"
            );
            moved_east += 1;
        }
        assert_eq!(
            moved_east, HAULER_COUNT,
            "all marked haulers must still exist and have drifted east"
        );
        return TestFlow::Screenshot("idle_spread".to_string());
    }

    TestFlow::Exit
}

/// Public entry point used by the harness. Mirrors the pattern in
/// `screenshots::defender_spread`.
#[allow(dead_code)]
pub fn run() -> Result<std::path::PathBuf, String> {
    run_screenshot_test(idle_spread)
}
