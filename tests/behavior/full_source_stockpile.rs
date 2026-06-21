//! Integration tests for issue #25: Handle full Source Stockpiles.
//!
//! The contract is the "Source Stockpile expansion" half of
//! the Source Stockpile flow:
//!
//!   1. A full nearby Source Stockpile is not treated as
//!      usable for new Worker delivery. The arrive system
//!      refuses to start extraction when the only nearby
//!      stockpile has `free_space() == 0`, and the carry-assign
//!      and delivery systems skip a full stockpile when
//!      picking a destination.
//!   2. If all nearby built Source Stockpiles are full and a
//!      valid placement exists, the demand system plans a new
//!      Planned Source Stockpile near the deposit.
//!   3. If an equivalent Planned Source Stockpile already
//!      exists (live or in the same tick), the demand system
//!      does not pile a duplicate plan on top.
//!   4. If no valid placement exists (every candidate blocked
//!      or out of the Gather Zone), no fallback Planned
//!      Source Stockpile is created.
//!   5. Workers can resume using the new Source Stockpile
//!      after it is built: extraction starts, the carried
//!      load is delivered, and the deposit drains.
//!
//! Each test pins one acceptance bullet so a failure points
//! at a single contract. The tests use the canonical
//! `sim_app_with_gather` and `sim_app_with_gather_planned`
//! builders from `tests/common/mod.rs` so the swarm and
//! plugin setup matches the rest of the suite.

use bevy::{math::Vec2, prelude::*};
use std::f32::consts::TAU;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        ExtractProgress, GatherAssignment, PlannedKind, PlannedStructure, SwarmId, WorkerLoad,
        SOURCE_STOCKPILE_JITTER_AMPLITUDE, SOURCE_STOCKPILE_PLACEMENT_COUNT,
        SOURCE_STOCKPILE_PLACEMENT_RADIUS, WORKER_CARRY_CAPACITY,
    },
    resources::{ResourceKind, Stockpile},
};

#[path = "../common/mod.rs"]
mod common;

/// Bot speed from the default game settings. Pulled into a
/// constant so the travel-time math in the test is obvious.
const BOT_SPEED: f32 = 5.0;

/// Distance the gather worker has to walk to reach the
/// planned Source Stockpile from the deposit (or back).
/// The demand system places the planned structure on the
/// placement ring at
/// [`SOURCE_STOCKPILE_PLACEMENT_RADIUS`] from the
/// deposit, plus a deterministic jitter of up to
/// [`SOURCE_STOCKPILE_JITTER_AMPLITUDE`]. The
/// travel-time math uses the worst case so the worker
/// has arrived by the time the test checks for the
/// completed build, regardless of the specific jitter
/// draw.
const PLANNED_TRAVEL_DISTANCE: f32 =
    SOURCE_STOCKPILE_PLACEMENT_RADIUS + SOURCE_STOCKPILE_JITTER_AMPLITUDE;

/// Ticks of simulation needed for the worker to walk
/// `distance` world units at `BOT_SPEED`. The arrival
/// is "distance / speed" rounded up to the next tick
/// because the movement system only prunes
/// `DirectMovementComponent` on the tick the bot
/// reaches its target.
fn travel_ticks(distance: f32) -> u32 {
    (distance / BOT_SPEED).ceil() as u32 + 1
}

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
        Some(SwarmId::PLAYER),
    ));
}

fn spawn_swarm_and_worker(app: &mut App, worker_pos: Vec2) -> (Entity, Entity) {
    let swarm = common::spawn_swarm_at(app, worker_pos);
    let worker = common::spawn_worker_at(app, worker_pos);
    (swarm, worker)
}

/// Number of Planned Source Stockpiles currently in the world.
fn planned_source_stockpile_count(world: &mut World) -> usize {
    world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SourceStockpile)
        .count()
}

/// Number of built Source Stockpiles (Stockpile entities) in
/// the world. A completed Source Stockpile is a regular
/// `Stockpile`, so this is the same query as
/// `stockpile_count` in the other Source Stockpile tests.
fn built_source_stockpile_count(world: &mut World) -> usize {
    world.query::<&Stockpile>().iter(world).count()
}

#[test]
fn full_source_stockpile_does_not_count_as_usable_for_extraction() {
    // Acceptance: "A full nearby Source Stockpile is not
    // treated as usable for new Worker delivery." The
    // gather arrive system checks for a *usable* built
    // Source Stockpile (free space > 0); a full stockpile
    // does not satisfy that check, so the worker must
    // not start extracting.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    // A full Source Stockpile right next to the deposit.
    // The arrive system must ignore it.
    let _full_stockpile = common::spawn_stockpile(
        &mut app,
        deposit_pos + Vec2::new(60.0, 0.0),
        /* amount */ 1000,
        /* capacity */ 1000,
    );

    // Drive long enough for: assignment, arrival at the
    // deposit, and several extract ticks. A worker that
    // *does* see the full stockpile as usable would start
    // extracting by the end of this loop.
    for _ in 0..20 {
        app.update();
    }

    let world = app.world_mut();
    let deposit_state = world
        .entity(deposit)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit_state.amount, 100,
        "full Source Stockpile must not satisfy the arrive system's usability check; \
         deposit must not drain"
    );
    let extract = world.entity(worker).get::<ExtractProgress>();
    assert!(
        extract.is_none(),
        "worker must not be in ExtractProgress while the only nearby stockpile is full"
    );
}

#[test]
fn full_source_stockpile_triggers_new_planned_source_stockpile() {
    // Acceptance: "If all nearby Source Stockpiles are full
    // and placement is valid, a new Planned Source
    // Stockpile is created." The demand system sees a
    // deposit whose only nearby built stockpile is full;
    // it plans a new Source Stockpile on the placement
    // ring (assuming the cell is open enough for a
    // candidate to pass the overlap filter).
    let mut app = common::sim_app_with_gather_planned();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    // A full Source Stockpile inside the cell. The demand
    // system must treat it as "not usable" and plan a
    // new one.
    let _full_stockpile = common::spawn_stockpile(
        &mut app,
        deposit_pos + Vec2::new(60.0, 0.0),
        /* amount */ 1000,
        /* capacity */ 1000,
    );

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned = planned_source_stockpile_count(world);
    assert_eq!(
        planned, 1,
        "demand system must plan a new Source Stockpile when the only nearby built one is full; \
         got {planned} planned Source Stockpiles"
    );
    // The original full stockpile still exists (the demand
    // system never removes built structures).
    let built = built_source_stockpile_count(world);
    assert_eq!(
        built, 1,
        "the full built Source Stockpile must still exist; got {built} built stockpiles"
    );
}

#[test]
fn full_source_stockpile_with_existing_planned_does_not_duplicate() {
    // Acceptance: "If an equivalent Planned Source
    // Stockpile already exists, duplicate plans are not
    // created." The demand system's "has any near Source
    // Stockpile" check treats the full built stockpile as
    // non-usable but a planned Source Stockpile as
    // demand-already-satisfied. A second pass on a
    // deposit that already has a planned Source Stockpile
    // must not pile a second plan.
    let mut app = common::sim_app_with_gather_planned();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    let _full_stockpile =
        common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(60.0, 0.0), 1000, 1000);
    // A pre-existing Planned Source Stockpile inside the
    // same cell. The cell center is the deposit's center,
    // so this plan is "near" the deposit. The demand
    // system must not plan a second one.
    let _planned = common::spawn_planned_structure_at_cell(&mut app, cell);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned = planned_source_stockpile_count(world);
    assert_eq!(
        planned, 1,
        "demand system must not pile a duplicate Planned Source Stockpile when one already exists; \
         got {planned} planned Source Stockpiles"
    );
}

#[test]
fn no_valid_placement_with_full_stockpile_creates_no_plan() {
    // Acceptance: "If no valid placement exists, no
    // fallback overlapping stockpile is created." The
    // Gather cell is painted, the deposit has a full
    // built stockpile, and every placement candidate
    // around the deposit is blocked by obstacles. The
    // demand system must not spawn a fallback Planned
    // Source Stockpile as an overlap.
    //
    // Uses `sim_app_with_gather` (no PlannedStructurePlugin)
    // so the only source of a Planned Source Stockpile is
    // the demand system itself. The pre-existing obstacles
    // are regular Stockpiles placed at each of the
    // ring's candidate positions; their half-footprint
    // (32) plus the placement padding (16) rejects every
    // candidate.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    // Full Source Stockpile well outside the ring (200
    // units east of the deposit, so it does not block
    // any ring candidate but is still "near" the
    // deposit from the proximity-radius point of view).
    let _full_stockpile =
        common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(200.0, 0.0), 1000, 1000);
    // Block every ring candidate with a pre-filled
    // stockpile at the ring position. Pre-filling is
    // important: an empty ring obstacle would also be a
    // usable Source Stockpile (`free_space > 0`) and
    // the carry-assign system would happily route the
    // worker to it, defeating the "no valid placement"
    // scenario. With `amount == capacity` the obstacle
    // still blocks placement but the carry-assign system
    // skips it, so the worker can neither extract (no
    // usable built stockpile) nor deliver (no usable
    // destination).
    let count = SOURCE_STOCKPILE_PLACEMENT_COUNT;
    let radius = SOURCE_STOCKPILE_PLACEMENT_RADIUS;
    for i in 0..count {
        let angle = i as f32 * (TAU / count as f32);
        let pos = deposit_pos + Vec2::new(angle.cos() * radius, angle.sin() * radius);
        let _ring_obstacle = common::spawn_stockpile(&mut app, pos, 1000, 1000);
    }
    // Hand-insert the GatherAssignment so the demand
    // system actually processes the deposit (the
    // assignment system would otherwise route the worker
    // to the gather cell, but the gather cell is
    // shared with the deposit so this is belt and
    // braces).
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(cell, deposit));

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned = planned_source_stockpile_count(world);
    assert_eq!(
        planned, 0,
        "demand system must not create a fallback Planned Source Stockpile when no valid placement exists; \
         got {planned} planned Source Stockpiles"
    );
    let deposit_state = world
        .entity(deposit)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit_state.amount, 100,
        "deposit must not drain when no valid placement exists and extraction cannot start"
    );
}

#[test]
fn worker_resumes_extraction_after_full_stockpile_expansion() {
    // Acceptance: "Workers can resume using the new Source
    // Stockpile after it is built." The full expansion
    // cycle: full built stockpile, demand system plans a
    // new one, Worker builds it, Worker resumes
    // extraction, the carried load is delivered to the
    // new stockpile, and the deposit drains.
    let mut app = common::sim_app_with_gather_planned();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    // Place the full stockpile well outside the ring so
    // it does not block any ring candidate -- the demand
    // system needs a free candidate to place a new plan.
    let _full_stockpile =
        common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(200.0, 0.0), 1000, 1000);

    // Drive long enough for: demand plan + travel to
    // planned + work + travel back to deposit + extract +
    // carry + travel to new stockpile + deliver.
    //
    // The travel math uses
    // `PLANNED_TRAVEL_DISTANCE = ring + jitter` so the
    // timing matches the worst-case jitter. With bot
    // speed 5.0 and distance 112, one leg is
    // (112/5).ceil() + 1 = 24 ticks. Total: 1 + 24 + 5
    // + 24 + 4 + 1 + 24 + 1 + buffer.
    let total_ticks = 1
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + top_down_2d_rts_prototype_nano_swarm::nanobot::DEFAULT_PLANNED_WORK_TICKS
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + WORKER_CARRY_CAPACITY
        + 1
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + 1
        + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let deposit_state = world
        .entity(deposit)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
        .unwrap();
    assert!(
        deposit_state.amount < 100,
        "Worker must resume extraction after the new Source Stockpile is built; \
         deposit drained from 100 to {}",
        deposit_state.amount
    );
    // The new Source Stockpile (started empty with capacity
    // 1000) received the delivered load, so its amount is
    // strictly between 0 and capacity. The pre-existing
    // full stockpile (amount == capacity) is excluded.
    let mut q = world.query::<&Stockpile>();
    let received_delivery = q
        .iter(world)
        .any(|stockpile| stockpile.amount > 0 && stockpile.amount < stockpile.capacity);
    assert!(
        received_delivery,
        "a new Source Stockpile must have received the delivered load"
    );
}

#[test]
fn worker_carry_assign_skips_full_stockpile() {
    // Acceptance: "A full nearby Source Stockpile is not
    // treated as usable for new Worker delivery." The
    // carry-assign system calls `find_nearest_stockpile`,
    // which already filters stockpiles with
    // `free_space() == 0`. A worker carrying a load with
    // only a full stockpile in range must not be assigned
    // a destination.
    let mut app = common::sim_app_with_gather();
    let worker_pos = Vec2::new(0.0, 0.0);
    let worker = common::spawn_worker_at(&mut app, worker_pos);
    // Stamp a `WorkerLoad` on the worker so the carry-assign
    // system processes them.
    app.world_mut().entity_mut(worker).insert(WorkerLoad {
        kind: ResourceKind::Minerals,
        amount: WORKER_CARRY_CAPACITY,
    });
    // Only a full stockpile in range.
    let _full_stockpile =
        common::spawn_stockpile(&mut app, worker_pos + Vec2::new(50.0, 0.0), 1000, 1000);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let returning = world
        .entity(worker)
        .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ReturningToStockpile>();
    assert!(
        returning.is_none(),
        "carry-assign must not pick a full Source Stockpile as a delivery destination"
    );
    // The load is still on the worker (the system drops the
    // destination, not the load, when no usable stockpile
    // exists).
    let load = world.entity(worker).get::<WorkerLoad>();
    assert!(
        load.is_some(),
        "worker must keep the load while waiting for a usable Source Stockpile"
    );
}

#[test]
fn worker_carry_assign_prefers_free_stockpile_over_full_one() {
    // The "skip full" filter is correct only when it does
    // not exclude a free stockpile that's also in range.
    // Two stockpiles in range: one full, one with free
    // space. The carry-assign system must pick the free
    // one.
    let mut app = common::sim_app_with_gather();
    let worker_pos = Vec2::new(0.0, 0.0);
    let worker = common::spawn_worker_at(&mut app, worker_pos);
    app.world_mut().entity_mut(worker).insert(WorkerLoad {
        kind: ResourceKind::Minerals,
        amount: WORKER_CARRY_CAPACITY,
    });
    // A full stockpile close to the worker.
    let _full_stockpile =
        common::spawn_stockpile(&mut app, worker_pos + Vec2::new(30.0, 0.0), 1000, 1000);
    // A free stockpile slightly further away.
    let free_stockpile =
        common::spawn_stockpile(&mut app, worker_pos + Vec2::new(60.0, 0.0), 0, 1000);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let returning = world
        .entity(worker)
        .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ReturningToStockpile>();
    let returning = returning.expect("carry-assign must pick the free stockpile");
    assert_eq!(
        returning.stockpile, free_stockpile,
        "carry-assign must prefer the free Source Stockpile over the full one"
    );
}

#[test]
fn worker_delivery_rejects_full_stockpile() {
    // Acceptance: "A full nearby Source Stockpile is not
    // treated as usable for new Worker delivery." The
    // delivery system checks `free_space()` against the
    // worker's load; a delivery that would overflow the
    // stockpile is rejected (the destination is dropped
    // so the carry-assign system can pick a different
    // one on the next tick; the load stays).
    let mut app = common::sim_app_with_gather();
    let stockpile_pos = Vec2::new(0.0, 0.0);
    let full_stockpile = common::spawn_stockpile(&mut app, stockpile_pos, 1000, 1000);
    let worker = common::spawn_worker_at(&mut app, stockpile_pos);
    // Stamp the worker with a load and a ReturningToStockpile
    // pointing at the full stockpile. The carry-assign
    // system is bypassed by hand-stamping the marker.
    app.world_mut().entity_mut(worker).insert((
        WorkerLoad {
            kind: ResourceKind::Minerals,
            amount: WORKER_CARRY_CAPACITY,
        },
        top_down_2d_rts_prototype_nano_swarm::nanobot::ReturningToStockpile {
            stockpile: full_stockpile,
        },
    ));

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let returning = world
        .entity(worker)
        .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ReturningToStockpile>();
    assert!(
        returning.is_none(),
        "delivery system must drop the destination when the only nearby stockpile is full"
    );
    // The load is still on the worker; the worker can pick
    // a different destination on a later tick (or wait
    // until a usable stockpile appears).
    let load = world.entity(worker).get::<WorkerLoad>();
    assert!(
        load.is_some(),
        "worker must keep the load after the delivery system drops a full-stockpile destination"
    );
    // The full stockpile's amount is unchanged.
    let stockpile = world.entity(full_stockpile).get::<Stockpile>().unwrap();
    assert_eq!(
        stockpile.amount, 1000,
        "full stockpile must not have received the load"
    );
}

#[test]
fn two_nearby_deposits_with_full_stockpiles_share_one_plan() {
    // Two deposits within the SOURCE_STOCKPILE_PROXIMITY_RADIUS
    // (120 + 16 jitter = 136, well under 384) each see only
    // a full Source Stockpile. The demand system processes
    // both deposits on the same tick: the first pass plans
    // a Source Stockpile, the second pass sees the in-tick
    // plan via `newly_planned_positions` and skips. The
    // swarm does not pile duplicate plans.
    let mut app = common::sim_app_with_gather_planned();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_a_pos = common::cell_world_center(cell);
    let deposit_b_pos = deposit_a_pos + Vec2::new(120.0, 0.0);
    let (_swarm, _worker_a) = spawn_swarm_and_worker(&mut app, deposit_a_pos);
    let _worker_b = common::spawn_worker_at(&mut app, deposit_b_pos);
    let _deposit_a = common::spawn_deposit(&mut app, deposit_a_pos, 100);
    let _deposit_b = common::spawn_deposit(&mut app, deposit_b_pos, 100);
    let _full_a =
        common::spawn_stockpile(&mut app, deposit_a_pos + Vec2::new(60.0, 0.0), 1000, 1000);
    let _full_b =
        common::spawn_stockpile(&mut app, deposit_b_pos + Vec2::new(60.0, 0.0), 1000, 1000);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned = planned_source_stockpile_count(world);
    assert_eq!(
        planned, 1,
        "two nearby deposits with only full stockpiles must share a single Planned Source Stockpile, \
         not pile duplicate plans; got {planned} planned Source Stockpiles"
    );
    // The single plan lands inside the painted Gather cell.
    let (planned_struct_cell, planned_pos) = world
        .query::<(&PlannedStructure, &Transform)>()
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::SourceStockpile)
        .map(|(p, t)| (p.cell, t.translation.truncate()))
        .expect("exactly one Planned Source Stockpile is asserted above");
    assert_eq!(
        planned_struct_cell, cell,
        "Planned Source Stockpile must live in the same cell as the deposits"
    );
    let planned_world_cell =
        top_down_2d_rts_prototype_nano_swarm::nanobot::world_to_cell(planned_pos);
    assert_eq!(
        planned_world_cell, cell,
        "Planned Source Stockpile world position must round to the same cell as the deposits"
    );
}
