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
//! The stranded-seek scene shows a small cluster of idle haulers starting
//! west of a Corridor segment and drifting east into it. The diagonal
//! crossing scene checks that a bot moves fully across both tile boundaries
//! instead of reversing and accumulating at their shared corner.
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
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, DirectMovementComponent, Health, Nanobot, NanobotType, StrategicController,
        SwarmId, SwarmMember, VelocityComponent, world_to_cell,
    },
};

use crate::harness::{
    TestContext, TestFlow, clear_nanobots_and_sprite_entities, run_screenshot_test,
};

/// Marker component tagging the haulers under test.
#[derive(Debug, Component)]
struct SpreadTestBot;

/// A swarm id that owns nothing in the default scenario. The haul
/// leg picker only matches legs whose source/sink owner is `None`
/// (unowned) or equals the hauler's swarm; the scenario stamps every
/// stockpile / facility / charger with a real owner, so this id has
/// no matching leg and its haulers are never grabbed.
const IDLE_SWARM: SwarmId = SwarmId(99);

/// Row south of the enclosed scenario
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
/// orthographic scale (2.0), which frames roughly five cells.
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
        // Give the isolated haulers their own Corridor intent.
        {
            let mut grid = world.resource_mut::<IntentGrid>();
            for &cell in &CORRIDOR_CELLS {
                grid.paint(cell, IntentKind::Corridor, IDLE_SWARM);
            }
        }
        // Spawn the idle haulers stacked at the stranded start cell.
        let spawn = cell_center(START_CELL);
        for _ in 0..HAULER_COUNT {
            world.spawn((
                SpreadTestBot,
                Nanobot {},
                NanobotType::Hauler,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(IDLE_SWARM),
                Transform::from_translation(spawn.extend(0.0)),
            ));
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

/// Public entry point used by the harness.
#[allow(dead_code)]
pub fn run() -> Result<std::path::PathBuf, String> {
    run_screenshot_test(idle_spread)
}

#[derive(Resource)]
struct DiagonalCrossingScene {
    bot: Entity,
    started_at: f64,
    crossed: bool,
}

/// Run with `cargo test --test screenshots -- --ignored idle_spread_diagonal_crossing`.
pub fn idle_spread_diagonal_crossing(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare_diagonal_crossing(ctx.world);
        return TestFlow::Screenshot("idle_spread_diagonal_start".into());
    }

    let scene = ctx.world.resource::<DiagonalCrossingScene>();
    let position = ctx
        .world
        .get::<Transform>(scene.bot)
        .unwrap()
        .translation
        .truncate();
    assert_eq!(
        ctx.world.get::<Commitment>(scene.bot),
        Some(&Commitment::Idle),
        "cosmetic spreading must preserve the idle commitment"
    );
    assert!(
        ctx.world
            .get::<DirectMovementComponent>(scene.bot)
            .is_none(),
        "cosmetic spreading must not create a movement task"
    );
    // Readback advances gameplay: completion records a crossing already observed,
    // rather than requiring the bot to remain at its capture-request position.
    if scene.crossed {
        return TestFlow::Exit;
    }
    let elapsed = ctx.world.resource::<Time<Fixed>>().elapsed_secs_f64() - scene.started_at;
    assert!(
        elapsed < 20.0,
        "idle hauler must cross fully into the diagonal tile within 20 simulated seconds; at {position} after {elapsed}s"
    );
    if position.x < -34.0 && position.y < -34.0 {
        assert_eq!(world_to_cell(position), IVec2::new(-1, -1));
        ctx.world.resource_mut::<DiagonalCrossingScene>().crossed = true;
        return TestFlow::Screenshot("idle_spread_diagonal_crossed".into());
    }
    TestFlow::Continue
}

fn prepare_diagonal_crossing(world: &mut World) {
    clear_nanobots_and_sprite_entities(world);
    for entity in world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        let _ = world.despawn(entity);
    }
    for entity in world
        .query_filtered::<Entity, With<StrategicController>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(entity).remove::<StrategicController>();
    }
    {
        let mut grid = world.resource_mut::<IntentGrid>();
        let mut existing_paint = Vec::new();
        for (point, cell) in grid.iter_active_cells() {
            for kind in IntentKind::ALL {
                for owner in cell.owners(kind) {
                    existing_paint.push((point, kind, owner));
                }
            }
        }
        for (point, kind, owner) in existing_paint {
            grid.erase(point, kind, owner);
        }
        for cell in [IVec2::ZERO, IVec2::new(-1, -1)] {
            for kind in [
                IntentKind::Defend,
                IntentKind::Gather,
                IntentKind::Build,
                IntentKind::Corridor,
            ] {
                grid.paint(cell, kind, SwarmId::PLAYER);
            }
        }
    }
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        zoom.zoom = 1.65;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 1.65;
        }
    }
    for (label, position) in [
        ("START TILE", Vec2::new(256.0, 430.0)),
        ("DIAGONAL TARGET", Vec2::new(-256.0, -430.0)),
    ] {
        world.spawn((
            Text2d::new(label),
            TextFont {
                font_size: 26.0,
                ..default()
            },
            Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z + 1.0)),
        ));
    }
    let bot = world
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            Commitment::Idle,
            Health::default(),
            VelocityComponent::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_xyz(72.0, 240.0, GAMEPLAY_SPRITE_Z + 1.0),
        ))
        .id();
    world.insert_resource(DiagonalCrossingScene {
        bot,
        started_at: world.resource::<Time<Fixed>>().elapsed_secs_f64(),
        crossed: false,
    });
}
