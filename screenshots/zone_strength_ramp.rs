//! Screenshot test for issue #40: encode paint strength into the zone
//! overlay alpha.
//!
//! Paints a grid of cells -- one row per [`IntentKind`], five columns at
//! increasing paint strengths -- and captures a frame so a human can read
//! the opacity ramp: strength 0 is invisible (absent), strength 1 reads
//! faint at the alpha floor, and a cell painted to [`PAINT_STRENGTH_CAP`]
//! reads solid at the alpha ceiling. The deterministic assertion proves
//! the data plumbing: every mirrored [`ZonePointData`] strength slot
//! equals the simulation value the brush wrote.
//!
//! Run: `cargo test --test screenshots -- --ignored zone_strength_ramp`
//! The artifact lands at
//! `target/playtest-screenshots/zone_strength_ramp.png` (gitignored with
//! `/target`).
//!
//! This test creates a real winit window + render pipeline, so it cannot
//! run headless. The unit test `strength_slots_do_not_corrupt_each_other`
//! in `src/zones/zone_brush.rs` and the `mouse_zone_painting` playtest
//! cover the data plumbing deterministically; this screenshot is the only
//! proof the visual alpha ramp actually renders.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::SwarmId,
    zones::{ZoneMaterial, ZoneMaterialHandleComponent},
    MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

use crate::harness::{TestContext, TestFlow};

/// Paint strength painted into each column, left to right. The first
/// column is `0` (absent / invisible), the last is the cap (solid).
/// `STRENGTHS[STRENGTHS.len() - 1]` must equal [`PAINT_STRENGTH_CAP`]
/// so the rightmost cell reads at the alpha ceiling.
const STRENGTHS: [u8; 5] = [0, 1, 4, 8, PAINT_STRENGTH_CAP];

/// Intent-grid X of each strength column, left to right. Five adjacent
/// cells so the ramp reads as one continuous horizontal band per kind.
const COLUMN_CELL_X: [i32; 5] = [-2, -1, 0, 1, 2];

/// Intent-grid Y of each kind's row, top (Gather) to bottom (Corridor).
/// The order matches [`IntentKind::ALL`] / the shader's slot order, and
/// the rows sit well clear of the default scenario's prepainted cells
/// along `Y = 0` so nothing else lands inside the framed region.
const ROW_CELL_Y: [i32; 4] = [6, 5, 4, 3];

/// Orthographic projection scale used to frame the cluster. Larger
/// scale shows more world units; this value fits the five-column by
/// four-row cluster into a 1280x720 window with a margin on each side
/// (`visible_world = window_pixels * scale`).
const FRAMING_SCALE: f32 = 3.2;

/// Frame on which the test stops warming up and asserts + captures. The
/// mirror system drains dirty cells every `Update`, so two frames after
/// the paint is more than enough for the GPU-side `ZonePointData` to
/// reflect the grid; eight frames is a comfortable margin that also lets
/// the render pipeline produce a stable frame before capture.
const CAPTURE_FRAME: u32 = 8;

pub fn zone_strength_ramp(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        let world = &mut *ctx.world;
        focus_camera(world);
        hide_existing_sprites(world);
        hide_existing_ui(world);
        paint_strength_ramp(world);
        return TestFlow::Continue;
    }

    if ctx.frame < CAPTURE_FRAME {
        return TestFlow::Continue;
    }

    if ctx.frame == CAPTURE_FRAME {
        assert_mirrored_strengths(ctx.world);
        return TestFlow::Screenshot("zone_strength_ramp".to_string());
    }

    TestFlow::Exit
}

/// Paint one row of cells per [`IntentKind`] at the strength ramp. Every
/// cell -- including the strength-`0` control column -- is written
/// through the same `add_owned` path the player brush and the
/// `spawn_opponent_swarm` helper use, stamped with player ownership so
/// the per-swarm intent contract holds. `add_owned` always marks the
/// cell dirty, so the mirror system visits every cell this frame.
fn paint_strength_ramp(world: &mut World) {
    let mut grid = world.resource_mut::<IntentGrid>();
    for (row_index, kind) in IntentKind::ALL.iter().enumerate() {
        for (column_index, strength) in STRENGTHS.iter().enumerate() {
            let cell = ramp_cell(row_index, column_index);
            grid.add_owned(cell, *kind, *strength, Some(SwarmId::PLAYER));
        }
    }
}

/// Assert the mirrored [`ZonePointData`] strength slots equal the
/// simulation values the brush wrote. This is the data-plumbing proof:
/// for every kind row and strength column, the slot in the GPU-side
/// material must match the strength the `IntentGrid` reports for the
/// same cell. The strength-`0` column asserts the absent contract
/// (slot `0` for an unpainted / strength-`0` kind).
fn assert_mirrored_strengths(world: &mut World) {
    let zone_handle = world
        .query::<&ZoneMaterialHandleComponent>()
        .iter(world)
        .next()
        .expect("the default scenario must spawn a ZoneMaterialHandleComponent on the camera")
        .handle
        .clone();
    let grid = world.resource::<IntentGrid>();
    let zone_mats = world.resource::<Assets<ZoneMaterial>>();
    let zone_mat = zone_mats
        .get(&zone_handle)
        .expect("the zone material handle must resolve to a live ZoneMaterial");

    for (row_index, kind) in IntentKind::ALL.iter().enumerate() {
        for (column_index, expected) in STRENGTHS.iter().enumerate() {
            let cell = ramp_cell(row_index, column_index);

            // Source of truth: the simulation grid.
            let sim_strength = grid
                .cell(cell)
                .unwrap_or_else(|| panic!("ramp cell {cell} must be in-bounds"))
                .strength(*kind);
            assert_eq!(
                sim_strength, *expected,
                "IntentGrid strength for {kind:?} at {cell} must match the painted ramp value"
            );

            // Mirror: the GPU-side material the shader reads.
            let buffer = buffer_index(cell);
            let mirror_slot = zone_mat.zone_data[buffer].strength(kind.index() as u32);
            assert_eq!(
                mirror_slot, *expected,
                "ZoneMaterial strength slot for {kind:?} at {cell} must mirror the simulation \
                 value; the shader's per-kind alpha ramp depends on this plumbing"
            );
        }
    }
}

/// The intent-grid cell for `kind_row` x `strength_column`.
fn ramp_cell(row_index: usize, column_index: usize) -> IVec2 {
    IVec2::new(COLUMN_CELL_X[column_index], ROW_CELL_Y[row_index])
}

/// Linear index into `ZoneMaterial::zone_data` for `cell`, applying the
/// same `y = height - y - 1` flip the mirror system uses (see
/// `zone_buffer_index_from_grid_point`). The grid is centered on the
/// origin and `MAP_WIDTH` x `MAP_HEIGHT` cells wide.
fn buffer_index(cell: IVec2) -> usize {
    let half_w = MAP_WIDTH as i32 / 2;
    let half_h = MAP_HEIGHT as i32 / 2;
    let mut idx = cell + IVec2::new(half_w, half_h);
    idx.y = MAP_HEIGHT as i32 - idx.y - 1;
    (idx.y as usize) * (MAP_WIDTH as usize) + (idx.x as usize)
}

/// Reposition and zoom the camera so the ramp cluster fills the frame.
/// Sets the projection scale and the `CameraZoom2d.zoom` field together
/// so they cannot disagree at runtime (the zoom system only rewrites the
/// projection on a mouse-wheel event, which this test never sends).
fn focus_camera(world: &mut World) {
    let center = cluster_world_center();
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = center.x;
        transform.translation.y = center.y;
        zoom.zoom = FRAMING_SCALE;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = FRAMING_SCALE;
        }
    }
}

/// World-space centre of the painted cluster, used to aim the camera.
/// Computed from the column / row cell layout so a layout change keeps
/// the cluster framed without a manual camera edit. Cell `c` occupies
/// world `[c, c+1) * ZONE_BLOCK_SIZE`, so the cluster spans the outer
/// cell edges and centres on their midpoint.
fn cluster_world_center() -> Vec2 {
    let min_x = COLUMN_CELL_X.iter().copied().min().unwrap();
    let max_x = COLUMN_CELL_X.iter().copied().max().unwrap();
    let min_y = ROW_CELL_Y.iter().copied().min().unwrap();
    let max_y = ROW_CELL_Y.iter().copied().max().unwrap();
    Vec2::new(
        (min_x + max_x + 1) as f32 * ZONE_BLOCK_SIZE / 2.0,
        (min_y + max_y + 1) as f32 * ZONE_BLOCK_SIZE / 2.0,
    )
}

/// Hide every `Sprite` entity the default scenario spawned (deposits,
/// facilities, nanobot sprites) so the captured frame shows only the
/// zone overlay ramp. The background and zone overlay quads are
/// `Mesh2d` entities, not `Sprite`, so they remain visible.
fn hide_existing_sprites(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Sprite>>()
        .iter(world)
        .collect();
    for entity in entities {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}

/// Hide UI panels and buttons that otherwise cover the ramp artifact.
fn hide_existing_ui(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect();
    for entity in entities {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
}
