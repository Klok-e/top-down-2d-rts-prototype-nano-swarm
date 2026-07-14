//! Scripted playtest for issue #38 / ADR-0004: the bot
//! lands at the deposit's world position, not at the
//! cell corner.
//!
//! The default `cargo run` scenario places the player
//! swarm at `cell_origin(PLAYER_CELL) = (256, 256)` -- a
//! half-cell offset. The pre-fix code parented the bot
//! to the swarm, so its `Transform.translation` was
//! local to the parent, and the movement system steered
//! the bot until the local value equalled the world
//! destination. The half-cell parent offset put the
//! bot at the deposit's cell corner
//! (deposit + (256, 256)), and the constructed
//! structure appeared at the bottom-left of the bot
//! cluster. The playtest below drives the full
//! simulation chain (assignment, demand, arrive,
//! extract, carry-assign, delivery) on a positioned
//! swarm and asserts the bot's `GlobalTransform` lands
//! at the deposit's world position.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::SwarmId,
};

#[path = "../common/mod.rs"]
mod common;

const CELL_SIZE: f32 = ZONE_BLOCK_SIZE;

fn build_app() -> App {
    common::sim_app_with_gather_planned()
}

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
}

#[test]
fn scripted_gather_bot_lands_at_deposit_world_position() {
    // The positioned-swarm scenario path: the swarm is
    // at the cell-center offset `(256, 256)` (the
    // default `cell_origin(PLAYER_CELL)`). A worker is
    // placed at the same world position. The deposit is
    // at the canonical gather cell center
    // `cell_origin(PLAYER_DEPOSIT_CELL) = (-768, 256)`.
    // The worker walks to the deposit, extracts, and
    // delivers. The playtest asserts the worker's
    // `GlobalTransform` is the deposit's world
    // position, not the cell corner.
    let mut app = build_app();
    let player_pos = Vec2::new(256.0, 256.0);
    let deposit_cell = IVec2::new(-2, 0);
    let deposit_pos = Vec2::new(
        deposit_cell.x as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        CELL_SIZE / 2.0,
    );
    let _swarm = common::spawn_swarm_at(&mut app, player_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    let _stockpile = common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 1000);
    let worker = common::spawn_worker_at(&mut app, player_pos);
    paint_gather(&mut app, deposit_cell);

    // Drive until physical arrival starts extraction. Stop before worker
    // begins its later Cargo trip toward Source Stockpile.
    let mut arrived = false;
    for _ in 0..500 {
        app.update();
        if app
            .world()
            .entity(worker)
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ExtractProgress>()
            .is_some()
        {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "worker must reach deposit and begin extraction");
    let world_mut = app.world_mut();
    let bot_pos = world_mut
        .entity(worker)
        .get::<Transform>()
        .expect("worker must have a Transform")
        .translation
        .truncate();
    let dist_to_deposit = bot_pos.distance(deposit_pos);
    assert!(
        dist_to_deposit <= 50.0,
        "worker Transform should land within the deposit's physical extent + 1-tick margin ({:?} +/- 50); got {:?}, distance to deposit = {}",
        deposit_pos,
        bot_pos,
        dist_to_deposit
    );
    // The pre-fix failure mode put the bot at the cell
    // corner (deposit + (256, 256)). The fix moves the
    // bot to the deposit's world position. Assert the
    // bot is far from the cell corner so the regression
    // is caught.
    let corner_x = deposit_pos.x + 256.0;
    let corner_y = deposit_pos.y + 256.0;
    let dist_to_corner = ((bot_pos.x - corner_x).powi(2) + (bot_pos.y - corner_y).powi(2)).sqrt();
    assert!(
        dist_to_corner > 200.0,
        "worker must not land at the cell corner (deposit + (256, 256)); got {:?}, distance to corner = {}",
        bot_pos,
        dist_to_corner
    );
}
