//! Integration tests for issue #22: visual overlap for Gather deposits.
//!
//! The pre-#22 gather contract picked a deposit when its center sat
//! in the same intent grid cell as the painted Gather paint. The
//! new contract uses geometric overlap: a deposit is eligible when
//! its circular work area (center, radius) intersects the
//! rectangle of a painted Gather cell owned by the worker's swarm.
//! Paint establishes geometric eligibility; candidate scoring resolves overlap.
//!
//! Each test pins one bullet of the acceptance criteria so a
//! failure points at a single contract:
//!
//! 1. `deposit_overlapping_painted_cell_is_eligible_when_center_in_other_cell`
//!    -- a deposit straddling the cell boundary is still eligible.
//! 2. `deposit_with_no_visual_overlap_remains_ineligible` -- a
//!    deposit far from the painted cell stays untouched.
//! 3. `deposit_overlapping_two_painted_cells_picks_nearest_paint`
//!    -- deterministic distance scoring resolves compatible overlap.
//! 4. `opponent_overlap_eligibility_does_not_leak_to_player_workers`
//!    -- a player Worker still cannot pick an opponent-painted
//!    deposit overlap (per-swarm ownership is preserved).

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{Commitment, GatherAssignment, SwarmId},
    resources::ResourceDeposit,
    ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn deposit_overlapping_painted_cell_is_eligible_when_center_in_other_cell() {
    // Paint Gather in cell (0, 0) (player-owned). Place a deposit
    // whose center is in cell (1, 0) but whose radius is large
    // enough to reach back into cell (0, 0). The deposit's circle
    // visually overlaps the painted cell's rectangle, so a player
    // worker placed in cell (0, 0) must receive a
    // GatherAssignment that points at this deposit. This is the
    // "eligible even when its center is not in the same intent
    // grid cell as the selected paint" acceptance bullet.
    let mut app = common::sim_app_with_gather();
    let painted_cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(painted_cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }
    // Deposit center sits in cell (1, 0) (cell x = 1 spans world
    // x = [512, 1024)). The chosen center (768, 256) is the cell
    // center. A radius of 300 makes the circle reach back into
    // cell (0, 0) (the closest point on cell (0, 0)'s rect is
    // (512, 256); distance 256 <= 300).
    let deposit_center = Vec2::new(768.0, ZONE_BLOCK_SIZE * 0.5);
    let deposit = common::spawn_deposit_with_radius(&mut app, deposit_center, 100, 300.0);
    // Worker at the painted cell center so scoring picks it.
    let worker_pos = common::cell_world_center(painted_cell);
    let worker = common::spawn_worker_at(&mut app, worker_pos);

    for _ in 0..5 {
        app.update();
    }

    let assignment = app
        .world()
        .entity(worker)
        .get::<GatherAssignment>()
        .expect("deposit whose circle overlaps the painted cell must be eligible");
    assert_eq!(
        assignment.deposit, deposit,
        "worker must be assigned to the overlapping deposit, not skipped"
    );
    assert_eq!(
        assignment.cell, painted_cell,
        "assignment still points at the painted cell the worker scored against"
    );
}

#[test]
fn deposit_with_no_visual_overlap_remains_ineligible() {
    // Paint Gather in cell (0, 0). Place a deposit whose center
    // is in cell (3, 0) with a small radius that does not reach
    // back into cell (0, 0). The deposit's circle does not
    // overlap the painted cell's rectangle, so the worker must
    // not receive a GatherAssignment. The deposit amount must
    // stay at its starting value (no extraction).
    let mut app = common::sim_app_with_gather();
    let painted_cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(painted_cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }
    // Deposit in cell (3, 0). Cell (3, 0) center is at
    // (3 * 512 + 256, 256) = (1792, 256). A radius of 32 stays
    // well within cell (3, 0); the closest point on cell (0, 0)'s
    // rect to (1792, 256) is (512, 256), distance 1280.
    let deposit_center = Vec2::new(1792.0, ZONE_BLOCK_SIZE * 0.5);
    let deposit = common::spawn_deposit_with_radius(&mut app, deposit_center, 100, 32.0);
    let worker_pos = common::cell_world_center(painted_cell);
    let worker = common::spawn_worker_at(&mut app, worker_pos);

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(worker)
            .get::<GatherAssignment>()
            .is_none(),
        "deposit with no visual overlap must not become a GatherAssignment target"
    );
    let deposit_state = app
        .world()
        .entity(deposit)
        .get::<ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit_state.amount, 100,
        "ineligible deposit must remain untouched (no extraction)"
    );
}

#[test]
fn deposit_overlapping_two_painted_cells_picks_nearest_paint() {
    // Binary paint makes both cells eligible; worker distance resolves choice.
    let mut app = common::sim_app_with_gather();
    let near_cell = IVec2::new(0, 0);
    let far_cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(near_cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
        assert!(grid.paint_owned(far_cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }
    // Deposit straddling the two cells. Center at (768, 256)
    // (cell (1, 0) center) with radius 300 reaches both cell
    // (0, 0) (distance 256) and cell (1, 0) (distance 0).
    let deposit_center = Vec2::new(768.0, ZONE_BLOCK_SIZE * 0.5);
    let deposit = common::spawn_deposit_with_radius(&mut app, deposit_center, 100, 300.0);
    // Worker is closer to cell (0, 0).
    let worker_pos = Vec2::new(300.0, ZONE_BLOCK_SIZE * 0.5);
    let worker = common::spawn_worker_at(&mut app, worker_pos);

    for _ in 0..5 {
        app.update();
    }

    let assignment = app
        .world()
        .entity(worker)
        .get::<GatherAssignment>()
        .expect("overlapping deposit must be eligible in both cells");
    assert_eq!(
        assignment.deposit, deposit,
        "worker must be assigned to the deposit that overlaps both cells"
    );
    assert_eq!(assignment.cell, near_cell);
}

#[test]
fn opponent_overlap_eligibility_does_not_leak_to_player_workers() {
    // A player-painted Gather cell exists, but a deposit's circle
    // overlaps an opponent-painted Gather cell. The player worker
    // must not pick the opponent cell even though the deposit's
    // geometry would make the cell "eligible" by the overlap
    // rule. This pins the per-swarm ownership contract from
    // issue #20 alongside the new overlap gate.
    let mut app = common::sim_app_with_gather();
    let opponent_id = SwarmId(11);
    let opponent_cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(opponent_cell, IntentKind::Gather, Some(opponent_id),));
    }
    // Deposit straddling the opponent cell with a generous
    // radius, so by the overlap rule it is a perfectly eligible
    // gather target. The per-swarm filter is the gate that keeps
    // it out of player workers' hands.
    let deposit_center = Vec2::new(300.0, ZONE_BLOCK_SIZE * 0.5);
    let deposit = common::spawn_deposit_with_radius(&mut app, deposit_center, 100, 300.0);
    let worker_pos = common::cell_world_center(opponent_cell);
    let worker = common::spawn_worker_at(&mut app, worker_pos);

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(worker)
            .get::<GatherAssignment>()
            .is_none(),
        "player worker must not pick an opponent-painted cell, even with a visually-overlapping deposit"
    );
    let deposit_state = app
        .world()
        .entity(deposit)
        .get::<ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit_state.amount, 100,
        "deposit must remain untouched because the player worker correctly skipped the opponent cell"
    );
    // Sanity: the worker is still idle (no other constraint
    // forces an assignment, and Commitment is the bundle
    // default).
    assert_eq!(
        *app.world().entity(worker).get::<Commitment>().unwrap(),
        Commitment::Idle,
    );
}
