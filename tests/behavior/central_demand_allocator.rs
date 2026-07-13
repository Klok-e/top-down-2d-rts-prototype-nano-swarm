//! Behavior tests for issue #35: central demand allocator with
//! Minimum Category Activation.
//!
//! Each test isolates one acceptance bullet so a failure points
//! at a single contract. The shared `sim_app_with_central_demand*`
//! builders wire the central allocator ahead of the per-category
//! assignment systems; tests assert ECS state directly through
//! `GatherAssignment` / `PlannedStructureClaim` markers, the
//! `DemandSnapshot` resource, and the `ActiveWorkerCounts`
//! resource.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, DemandCategory, GatherAssignment, PlannedKind, PlannedStructure,
        PlannedStructureClaim, SwarmId,
    },
};

#[path = "../common/mod.rs"]
mod common;

/// Paint `cell` with player-owned Gather intent.
fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
}

#[test]
fn demand_snapshot_reflects_painted_gather_zone() {
    // Acceptance: "Work systems publish or expose demand
    // items by swarm and category, including at least
    // Gather work".
    //
    // The central allocator must observe the freshly
    // painted Gather cell, the deposit overlapping it,
    // and surface a `DemandItem` per (swarm, category)
    // pair in the snapshot.
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);

    app.update();

    let snap = common::read_demand_snapshot(&app);
    assert!(
        snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather),
        "DemandSnapshot must include a Gather demand for the player swarm"
    );
    let items = snap.for_swarm(SwarmId::PLAYER);
    let gather_count = items
        .iter()
        .filter(|i| i.category == DemandCategory::Gather)
        .count();
    assert_eq!(gather_count, 1, "exactly one Gather demand item expected");
    assert_eq!(items[0].cell, cell);
}

#[test]
fn paint_gather_zone_pulls_idle_worker_through_central_allocator() {
    // Acceptance: "Painting a new valid Gather Zone while
    // workers are idle or overcommitted elsewhere produces
    // visible Worker response promptly." The first
    // allocation must go through the central allocator's
    // Gather minimum-activation path -- a `GatherAssignment`
    // plus a `DirectMovementComponent` pointing at the
    // deposit, not via the per-category
    // `worker_gather_assignment_system` (which is not even
    // registered in this builder).
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let assignment = world
        .entity(worker)
        .get::<GatherAssignment>()
        .expect("central allocator must produce a GatherAssignment for the idle worker");
    assert_eq!(assignment.cell, cell);
    assert_eq!(
        *world.entity(worker).get::<Commitment>().unwrap(),
        Commitment::Idle,
        "the central allocator's claim does not change the worker's commitment"
    );
    let active = common::read_active_worker_counts(&app);
    assert_eq!(
        active.count(SwarmId::PLAYER, DemandCategory::Gather),
        1,
        "active worker count for (PLAYER, Gather) must reflect the claim"
    );
}

#[test]
fn no_gather_demand_leaves_idle_workers_unassigned() {
    // Acceptance: "No fake work is created: if a category
    // has no valid demand or no eligible idle nanobot,
    // the allocator leaves it unassigned."
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    assert!(
        world.entity(worker).get::<GatherAssignment>().is_none(),
        "no Gather paint + no deposit => no GatherAssignment"
    );
    assert!(
        world
            .entity(worker)
            .get::<PlannedStructureClaim>()
            .is_none(),
        "no planned structure => no PlannedStructureClaim"
    );
    let snap = common::read_demand_snapshot(&app);
    assert!(!snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather));
    assert!(!snap.has_demand(SwarmId::PLAYER, DemandCategory::PlannedBuild));
}

#[test]
fn no_eligible_idle_leaves_demand_unassigned() {
    // Acceptance: "if a category has no eligible idle
    // nanobot, the allocator leaves it unassigned."
    // A Hauler is the only nanobot in the swarm; it is
    // not eligible for the Worker-driven categories.
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    let hauler = common::spawn_hauler_at(&mut app, center);

    app.update();

    let world = app.world();
    assert!(
        world.entity(hauler).get::<GatherAssignment>().is_none(),
        "hauler must not be claimed for a Worker-only category"
    );
    let snap = common::read_demand_snapshot(&app);
    assert!(
        snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather),
        "demand must still be observed even when no eligible bot exists"
    );
    let active = common::read_active_worker_counts(&app);
    assert_eq!(
        active.count(SwarmId::PLAYER, DemandCategory::Gather),
        0,
        "no claim issued: active count must stay at zero"
    );
}

#[test]
fn already_active_category_is_not_re_activated() {
    // Acceptance: "if a category has ... zero active
    // workers ... at least one nanobot is assigned
    // promptly." A category with an active worker is
    // already at the minimum, so the allocator does not
    // issue a second claim.
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let deposit = common::spawn_deposit(&mut app, center, 100);
    // A first worker is already on its way to the
    // deposit. The allocator must not issue a second
    // GatherAssignment.
    let first_worker = common::spawn_worker_at(&mut app, center);
    app.world_mut().entity_mut(first_worker).insert((
        GatherAssignment::new(cell, deposit),
        top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent {
            xy: center,
            // Extent-less destination for the
            // pre-claim path the test fixtures
            // build by hand.
            stop_radius: 0.0,
        },
    ));
    let second_worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    assert!(
        world
            .entity(second_worker)
            .get::<GatherAssignment>()
            .is_none(),
        "second worker must not be claimed when the category already has an active worker"
    );
    let active = common::read_active_worker_counts(&app);
    assert_eq!(
        active.count(SwarmId::PLAYER, DemandCategory::Gather),
        1,
        "active count must reflect the pre-existing worker, not double-count"
    );
}

#[test]
fn unclaimed_planned_structure_pulls_idle_worker() {
    // Acceptance: "A Planned Structure with an available
    // Worker gets at least one builder promptly." The
    // central allocator's Planned Build min-activation
    // path must pre-claim one worker for an unclaimed
    // planned structure, even when no per-category plugin
    // is registered.
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        cell,
        PlannedKind::SourceStockpile,
    );
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let claim = world
        .entity(worker)
        .get::<PlannedStructureClaim>()
        .expect("central allocator must produce a PlannedStructureClaim for the idle worker");
    assert_eq!(claim.cell, cell);
    assert_eq!(claim.target, planned);
    let planned_state = world.entity(planned).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned_state.active_worker,
        Some(worker),
        "the planned structure's active_worker must point at the claimed worker"
    );
    let active = common::read_active_worker_counts(&app);
    assert_eq!(
        active.count(SwarmId::PLAYER, DemandCategory::PlannedBuild),
        1
    );
}

#[test]
fn claimed_planned_structure_is_not_a_demand_item() {
    // Acceptance: "No fake work is created". A planned
    // structure that already has a worker must not be
    // visible as a demand item to the central allocator.
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let planned = common::spawn_planned_structure_of_kind_at_cell(
        &mut app,
        cell,
        PlannedKind::SourceStockpile,
    );
    let first_worker = common::spawn_worker_at(&mut app, center);
    app.world_mut()
        .entity_mut(first_worker)
        .insert(PlannedStructureClaim {
            cell,
            target: planned,
        });
    let second_worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    assert!(
        world
            .entity(second_worker)
            .get::<PlannedStructureClaim>()
            .is_none(),
        "second worker must not be claimed when the planned structure is already taken"
    );
    let active = common::read_active_worker_counts(&app);
    assert_eq!(
        active.count(SwarmId::PLAYER, DemandCategory::PlannedBuild),
        1,
        "the existing claim must count, and the second worker must not be claimed"
    );
}

#[test]
fn demand_snapshot_excludes_other_swarm_owned_paint() {
    // Per-swarm intent ownership contract (issue #20).
    // A cell painted by opponent swarm B must not be
    // visible to the player swarm's central allocator,
    // so the player sees no demand from B's paint.
    let mut app = common::sim_app_with_central_demand();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    // Player swarm present so the central allocator
    // publishes player snapshots.
    let _player_swarm = common::spawn_swarm_at(&mut app, center);
    // Opponent paint on the same cell with a deposit
    // would be a Gather demand for the opponent. The
    // player's snapshot must not include it.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId(7)),));
    }
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let snap = common::read_demand_snapshot(&app);
    assert!(
        !snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather),
        "player swarm must not see opponent-owned Gather paint"
    );
    let world = app.world();
    assert!(
        world.entity(worker).get::<GatherAssignment>().is_none(),
        "no player worker should be claimed for opponent demand"
    );
}

#[test]
fn central_allocator_claims_gather_before_per_category_system_runs() {
    // Acceptance: "Idle eligible nanobots are assigned
    // through one central allocator path rather than
    // each category independently stealing workers
    // first." This test wires the full
    // gather + planned-structure + central-demand
    // builder and confirms that the central allocator
    // (not the per-category gather system) is the one
    // that produces the first assignment on tick 0 of
    // a freshly painted Gather cell.
    //
    // The two paths insert the same `GatherAssignment`
    // marker, so the test cannot distinguish them by
    // marker alone. Instead, it asserts that the
    // snapshot and active counts match the central
    // allocator's pre-claim by tick 1 -- which is the
    // "promptly" half of the contract.
    let mut app = common::sim_app_with_central_demand_gather();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    paint_gather(&mut app, cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    let _worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let active = common::read_active_worker_counts(&app);
    assert_eq!(
        active.count(SwarmId::PLAYER, DemandCategory::Gather),
        1,
        "by tick 1 the central allocator must have produced the first Gather claim"
    );
    let snap = common::read_demand_snapshot(&app);
    assert!(snap.has_demand(SwarmId::PLAYER, DemandCategory::Gather));
}
