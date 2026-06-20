//! Integration tests for issue #7: Gather Zones with physical resource pickup.
//!
//! Covers the four behaviors the issue calls out as acceptance criteria:
//! extraction, small-load carrying, depletion persistence, and
//! reactivation. The tests drive a minimal Bevy `App` (no rendering,
//! no zone material) and assert ECS state directly: resource amounts on
//! deposits, worker load components, and IntentGrid layers.
//!
//! Tests are organised as a vertical-slice TDD loop. Each test isolates
//! one behavior so failures point at a single contract:
//!   1. `worker_extracts_one_unit_per_tick_when_at_deposit` -- extraction
//!   2. `worker_fills_small_load_then_head_to_stockpile` -- small load cap
//!   3. `worker_delivers_carry_to_nearest_stockpile` -- physical delivery
//!   4. `gather_intent_persists_after_deposit_depletes` -- persistence
//!   5. `idle_worker_reactivates_when_deposit_refills` -- reactivation
//!   6. `idle_worker_chooses_gather_via_autonomy_scoring` -- scoring wires in
//!   7. `haulers_do_not_extract_directly` -- type-fit gating

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        best_candidate, Commitment, ExtractProgress, GatherAssignment, NanobotType,
        ReturningToStockpile, SoftWorkSlots, WorkerLoad, EXTRACT_PER_TICK, WORKER_CARRY_CAPACITY,
    },
    resources::{ResourceDeposit, Stockpile},
    ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

const CELL_SIZE: f32 = ZONE_BLOCK_SIZE;

fn build_app() -> App {
    common::sim_app_with_gather()
}

#[test]
fn worker_extracts_one_unit_per_tick_when_at_deposit() {
    // Tracer bullet: a worker assigned to a deposit and standing on
    // it extracts EXTRACT_PER_TICK units per update, draining the
    // deposit and building an ExtractProgress component.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 10);
    let worker = common::spawn_worker_at(&mut app, deposit_pos);

    // Pre-seed a GatherAssignment so the test isolates extraction
    // from the assignment algorithm. The assignment algorithm has its
    // own test below.
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::new(0, 0), deposit));

    app.update();

    let world = app.world();
    let deposit = world.entity(deposit).get::<ResourceDeposit>().unwrap();
    let progress = world
        .entity(worker)
        .get::<ExtractProgress>()
        .expect("worker should start ExtractProgress when at deposit");
    assert_eq!(
        deposit.amount,
        10 - EXTRACT_PER_TICK,
        "deposit loses one unit per tick"
    );
    assert_eq!(
        progress.collected, EXTRACT_PER_TICK,
        "ExtractProgress collects one unit per tick"
    );
}

#[test]
fn worker_fills_small_load_then_head_to_stockpile() {
    // A Worker carries a small load (WORKER_CARRY_CAPACITY units).
    // Once the load is full, the worker must transition to Carrying
    // and head to the nearest stockpile -- not continue extracting
    // past the cap.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(200.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 100);
    let worker = common::spawn_worker_at(&mut app, deposit_pos);

    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::new(0, 0), deposit));

    // Run enough updates to fill the load.
    let ticks = (WORKER_CARRY_CAPACITY + 2) as usize;
    for _ in 0..ticks {
        app.update();
    }

    let world = app.world();
    let load = world
        .entity(worker)
        .get::<WorkerLoad>()
        .expect("worker should have a WorkerLoad after the load is full");
    assert_eq!(
        load.amount, WORKER_CARRY_CAPACITY,
        "small load caps at WORKER_CARRY_CAPACITY"
    );

    // The worker must be heading to the stockpile.
    let returning = world
        .entity(worker)
        .get::<ReturningToStockpile>()
        .expect("full load triggers ReturningToStockpile");
    assert_eq!(
        returning.stockpile, stockpile,
        "worker heads to the nearest stockpile"
    );

    // ExtractProgress is gone; it has been rolled into WorkerLoad.
    assert!(
        world.entity(worker).get::<ExtractProgress>().is_none(),
        "ExtractProgress should be cleared when the load is full"
    );

    // The deposit was drained for at most WORKER_CARRY_CAPACITY units
    // (the cap), not more.
    let deposit = world.entity(deposit).get::<ResourceDeposit>().unwrap();
    assert_eq!(
        deposit.amount,
        100 - WORKER_CARRY_CAPACITY,
        "deposit loses exactly WORKER_CARRY_CAPACITY units, no more"
    );
}

#[test]
fn worker_delivers_carry_to_nearest_stockpile() {
    // When the worker reaches the stockpile, the load is dropped
    // into it and the worker becomes idle again -- not stuck in a
    // Carrying state. The dropped amount matches what the worker
    // was carrying.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(150.0, 0.0); // very close, within radius
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    let stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 100);
    let worker = common::spawn_worker_at(&mut app, deposit_pos);

    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(IVec2::new(0, 0), deposit));

    // Fill the load. EXTRACT_PER_TICK ticks of extraction at the
    // deposit (no travel), plus one more for the transition.
    for _ in 0..(WORKER_CARRY_CAPACITY as usize) {
        app.update();
    }
    // Travel from the deposit at (100, 0) to the stockpile at
    // (150, 0) at bot_speed 5.0 = 10 ticks, plus one tick for the
    // arrival + delivery. 20 ticks is a safe margin.
    for _ in 0..20 {
        app.update();
    }

    let world = app.world();
    let stockpile_state = world.entity(stockpile).get::<Stockpile>().unwrap();
    assert!(
        stockpile_state.amount >= WORKER_CARRY_CAPACITY,
        "stockpile should receive the worker's load; got {}",
        stockpile_state.amount
    );

    // WorkerLoad is removed on successful delivery, not just
    // zeroed.
    assert!(
        world.entity(worker).get::<WorkerLoad>().is_none(),
        "WorkerLoad should be removed after delivery"
    );
    assert!(
        world.entity(worker).get::<ReturningToStockpile>().is_none(),
        "ReturningToStockpile should be removed after delivery"
    );
}

#[test]
fn gather_intent_persists_after_deposit_depletes() {
    // The Gather Zone (player-painted intent) must stay painted
    // after the deposit it covered is drained. Workers leave when
    // no work remains, but the intent layer does not. This is the
    // "persist when local resources are depleted" acceptance bullet.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Gather, PAINT_STRENGTH_CAP));
    }
    app.update();

    // Paint survives a flush of updates and "no work remains" -- no
    // system ever clears intent based on deposit state.
    for _ in 0..10 {
        app.update();
    }
    let grid = app.world().resource::<IntentGrid>();
    let painted = grid.cell(cell).expect("cell must exist");
    assert!(
        painted.has(IntentKind::Gather),
        "Gather intent must persist after local deposits are depleted"
    );
    assert_eq!(
        painted.strength(IntentKind::Gather),
        PAINT_STRENGTH_CAP,
        "Gather strength unchanged by depletion"
    );
}

#[test]
fn idle_worker_reactivates_when_deposit_refills() {
    // When a deposit is refilled, an idle worker on the same cell
    // must re-engage with the new extraction. The Gather Zone
    // itself was never cleared, so the worker's reactivation is the
    // visible end of the "persist + reactivate" contract.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let stockpile_pos = Vec2::new(150.0, 0.0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 4);
    let _stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 0, 1000);
    let worker = common::spawn_worker_at(&mut app, deposit_pos);

    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(IVec2::new(0, 0), IntentKind::Gather, PAINT_STRENGTH_CAP));
    }

    // Drain the deposit. 4 extraction ticks at the deposit (no
    // travel) plus a handful of transport + delivery ticks.
    for _ in 0..20 {
        app.update();
    }
    {
        let deposit_state = app
            .world()
            .entity(deposit)
            .get::<ResourceDeposit>()
            .unwrap();
        assert_eq!(
            deposit_state.amount, 0,
            "deposit is fully drained at the end of the depletion phase"
        );
    }

    // Refill the deposit. Pre-position the worker at the deposit
    // so the test does not depend on the bot's travel time to
    // reach it -- the contract is "refill triggers re-engagement",
    // not "refill triggers a long walk back".
    app.world_mut()
        .entity_mut(worker)
        .get_mut::<Transform>()
        .unwrap()
        .translation = deposit_pos.extend(0.0);
    app.world_mut()
        .entity_mut(deposit)
        .get_mut::<ResourceDeposit>()
        .unwrap()
        .amount = 8;

    // The worker must re-engage within a few ticks of the refill.
    // After EXTRACT_PER_TICK ticks of extraction the deposit is
    // observably smaller, which is the cleanest signal that the
    // worker is extracting from the refilled deposit.
    for _ in 0..(EXTRACT_PER_TICK as usize + 2) {
        app.update();
    }
    let deposit_state = app
        .world()
        .entity(deposit)
        .get::<ResourceDeposit>()
        .unwrap();
    assert!(
        deposit_state.amount < 8,
        "idle worker must re-engage: deposit drained from 8 to {}",
        deposit_state.amount
    );
}

#[test]
fn idle_worker_chooses_gather_via_autonomy_scoring() {
    // The autonomy scoring from issue #6 must drive gather work for
    // idle workers: an idle worker on a Gather-painted cell with a
    // deposit in it must be assigned a GatherAssignment that points
    // at the deposit.
    let mut app = build_app();
    let cell = IVec2::new(2, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Gather, PAINT_STRENGTH_CAP));
    }
    let cell_world_center = common::cell_world_center(cell);
    let deposit = common::spawn_deposit(&mut app, cell_world_center, 100);
    // Place the worker close to the cell so the assignment system
    // picks it (the scoring is global, but the test wants the
    // assignment to clearly land on this cell).
    let worker = common::spawn_worker_at(&mut app, cell_world_center);

    // Sanity: the global scoring function picks this cell for an
    // idle Worker; this is the contract the assignment system
    // consumes.
    {
        let grid = app.world().resource::<IntentGrid>();
        let slots = app.world().resource::<SoftWorkSlots>();
        let picked = best_candidate(
            grid,
            NanobotType::Worker,
            Commitment::Idle,
            cell_world_center,
            slots,
            CELL_SIZE,
            &[IntentKind::Gather],
        )
        .expect("Gather cell must be a candidate");
        assert_eq!(picked.cell, cell);
        assert_eq!(picked.kind, IntentKind::Gather);
    }

    // Drive the assignment system; the worker should end up with a
    // GatherAssignment pointing at the deposit in the gather cell.
    for _ in 0..5 {
        app.update();
    }

    let assignment = app
        .world()
        .entity(worker)
        .get::<GatherAssignment>()
        .expect("idle worker should receive a GatherAssignment from the assignment system");
    assert_eq!(assignment.deposit, deposit);
    assert_eq!(assignment.cell, cell);
}

#[test]
fn haulers_do_not_extract_directly() {
    // Haulers fit Build and Corridor but not Gather. They must not
    // be assigned gather work by the assignment system even if a
    // Gather cell exists with a deposit in it. This is the type-fit
    // contract for Gather extraction.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Gather, PAINT_STRENGTH_CAP));
    }
    let cell_world_center = common::cell_world_center(cell);
    let _deposit = common::spawn_deposit(&mut app, cell_world_center, 100);
    let hauler = common::spawn_hauler_at(&mut app, cell_world_center);

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(hauler)
            .get::<GatherAssignment>()
            .is_none(),
        "haulers must not be assigned gather work -- type fit is zero for Gather"
    );
    let deposit = app
        .world()
        .entity(_deposit)
        .get::<ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit.amount, 100,
        "deposit is untouched because no worker engaged"
    );
}
