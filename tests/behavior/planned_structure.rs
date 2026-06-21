//! Integration tests for issue #21: Planned Structure foundation.
//!
//! Each test isolates one behaviour so a failure points at a single
//! acceptance bullet. The slice is "foundation + one demo kind":
//! the demo kind is Source Stockpile, the lifecycle is visible
//! planning, one-worker reservation, worker-time progress, and
//! completion into a real `Stockpile`. V1 build work consumes no
//! minerals, so the resource ledger stays flat across the build.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        completed_visual_color, planned_visual_color, OwnerSwarm, PlannedStructure,
        PlannedStructureClaim, PlannedStructureProgress, Swarm, DEFAULT_PLANNED_WORK_TICKS,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
    ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_planned()
}

#[test]
fn planned_structure_emerges_in_build_painted_cell() {
    // Acceptance: "Planned Structures are visibly distinct from
    // completed structures" + "visible immediately". Painting a
    // Build cell must cause a PlannedStructure to appear with
    // the planned visual on the same tick the auto-creation
    // system runs.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Sprite, &Transform)>();
    let (planned, sprite, transform) = q
        .iter(world)
        .next()
        .expect("PlannedStructure must spawn in a Build-painted cell");
    assert_eq!(
        planned.kind,
        top_down_2d_rts_prototype_nano_swarm::nanobot::PlannedKind::SourceStockpile
    );
    assert_eq!(planned.cell, cell);
    // The visual is the planned color so the player can tell
    // the structure is not yet built.
    assert_eq!(
        sprite.color,
        planned_visual_color(),
        "PlannedStructure must use the planned visual color"
    );
    let center = common::cell_world_center(cell);
    assert!(
        (transform.translation.truncate() - center).length() < 1.0,
        "PlannedStructure must be created at the cell's world center; got {:?}",
        transform.translation
    );
}

#[test]
fn planned_structure_not_duplicated_when_one_already_exists() {
    // Acceptance: "automatic construction" must not pile
    // multiple planned structures into the same cell. Once a
    // cell has a PlannedStructure, repeated ticks must not
    // spawn another.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<&PlannedStructure>();
    let count = q.iter(world).count();
    assert_eq!(
        count, 1,
        "auto-creation must not duplicate PlannedStructures"
    );
}

#[test]
fn planned_structure_not_emerged_for_gather_only_cell() {
    // Build intent is the demand layer for the planned-structure
    // foundation. A Gather-only cell does not express
    // construction demand, so no PlannedStructure emerges.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<&PlannedStructure>();
    let count = q.iter(world).count();
    assert_eq!(
        count, 0,
        "Gather-only cell must not spawn a PlannedStructure"
    );
}

#[test]
fn idle_worker_claims_one_unclaimed_planned_structure() {
    // Acceptance: "A Worker can claim one unclaimed Planned
    // Structure." An idle worker placed in a Build cell with
    // a single planned structure must receive a claim and
    // become the planned structure's `active_worker`.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let claim = world
        .entity(worker)
        .get::<PlannedStructureClaim>()
        .expect("idle worker must claim the planned structure");
    assert_eq!(claim.target, planned);
    let planned_state = world.entity(planned).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned_state.active_worker,
        Some(worker),
        "planned structure must record the worker as active_worker"
    );
}

#[test]
fn only_one_worker_can_claim_a_planned_structure() {
    // Acceptance: "Other Workers do not work on an already
    // claimed Planned Structure." Two idle workers and one
    // planned structure: only one worker ends up with a
    // claim; the other stays idle.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    let worker_a = common::spawn_worker_at(&mut app, center);
    let worker_b = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let planned_state = world.entity(planned).get::<PlannedStructure>().unwrap();
    let active = planned_state
        .active_worker
        .expect("the planned structure must be claimed");
    assert!(
        active == worker_a || active == worker_b,
        "the active worker must be one of the two idle workers"
    );
    let claim_count_a = world
        .entity(worker_a)
        .get::<PlannedStructureClaim>()
        .is_some();
    let claim_count_b = world
        .entity(worker_b)
        .get::<PlannedStructureClaim>()
        .is_some();
    let claim_count = (claim_count_a as u32) + (claim_count_b as u32);
    assert_eq!(
        claim_count, 1,
        "exactly one worker must hold the claim; got a={} b={}",
        claim_count_a, claim_count_b
    );
}

#[test]
fn claimed_planned_structure_is_skipped_by_other_workers() {
    // Acceptance: "Other Workers do not work on an already
    // claimed Planned Structure." A second worker that wakes
    // up after the first has already claimed must NOT take
    // the same planned structure. We model the late claim by
    // stamping the reservation manually and then running the
    // claim system on a second idle worker.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    // Pre-claim the planned structure. The real claim system
    // would do this, but we want to focus the test on the
    // "skip claimed" half of the contract.
    let claiming_worker = common::spawn_worker_at(&mut app, center);
    {
        let world = app.world_mut();
        let mut state = *world.entity(planned).get::<PlannedStructure>().unwrap();
        state.active_worker = Some(claiming_worker);
        world.entity_mut(planned).insert(state);
    }
    // Late worker that tries to claim after the first.
    let late_worker = common::spawn_worker_at(&mut app, center);

    for _ in 0..3 {
        app.update();
    }

    let world = app.world();
    let late_claim = world.entity(late_worker).get::<PlannedStructureClaim>();
    assert!(
        late_claim.is_none(),
        "late worker must not be able to claim a planned structure that is already reserved"
    );
    let late_progress = world.entity(late_worker).get::<PlannedStructureProgress>();
    assert!(
        late_progress.is_none(),
        "late worker must not be in progress on a reserved planned structure"
    );
    let planned_state = world.entity(planned).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned_state.active_worker,
        Some(claiming_worker),
        "reservation must be preserved across later claim attempts"
    );
}

#[test]
fn worker_time_advances_build_progress() {
    // Acceptance: "Worker time advances build progress." A
    // worker that has arrived at a planned structure must
    // see the `work_remaining` counter drop by 1 each tick
    // it spends in the working state.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    // Place the worker inside the planned structure's stop
    // threshold so the arrive system promotes it to progress
    // on the same tick the claim system fires.
    let _worker = common::spawn_worker_at(&mut app, center);

    // 1 tick for claim + arrive (worker is already at the
    // cell center), then DEFAULT ticks to consume the budget.
    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    // The planned structure is either still in flight (work
    // remaining > 0) or has been promoted (no PlannedStructure
    // component on the entity). Both are valid post-build
    // states; the contract is "work_remaining decreases while
    // a worker is in the progress state".
    let still_planned = world.entity(planned).get::<PlannedStructure>().copied();
    if let Some(state) = still_planned {
        assert!(
            state.work_remaining < DEFAULT_PLANNED_WORK_TICKS,
            "work_remaining must decrease as the worker spends time; got {}",
            state.work_remaining
        );
    }
}

#[test]
fn completion_replaces_planned_with_stockpile() {
    // Acceptance: "Completion replaces the Planned Structure
    // with the appropriate completed structure for the demo
    // kind." A planned structure that reaches 0 work
    // remaining is replaced with a `Stockpile` carrying
    // the Source Stockpile demo shape, and the visual flips
    // to the completed color.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    let _worker = common::spawn_worker_at(&mut app, center);

    // 1 tick for claim + arrive, then the work budget, then
    // a buffer. The build should be done well before the loop
    // ends.
    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let planned_after = world.entity(planned).get::<PlannedStructure>();
    assert!(
        planned_after.is_none(),
        "PlannedStructure must be removed on completion"
    );
    let stockpile = world
        .entity(planned)
        .get::<Stockpile>()
        .expect("completion must replace PlannedStructure with a Stockpile");
    assert_eq!(stockpile.kind, ResourceKind::Minerals);
    assert_eq!(stockpile.amount, 0);
    let sprite = world
        .entity(planned)
        .get::<Sprite>()
        .expect("completed structure must carry a Sprite for the visual flip");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed visual must use the completed color"
    );
}

#[test]
fn completion_does_not_consume_any_minerals() {
    // Acceptance: "V1 build work consumes no minerals or other
    // materials." The resource ledger must be empty across the
    // full build: nothing was pulled from any stockpile and
    // nothing was added to the swarm-wide totals.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    // No stockpiles anywhere. The build must not require any
    // material source; if it did, the build would block on an
    // empty ledger and never complete.
    let _worker = common::spawn_worker_at(&mut app, center);

    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let ledger = app.world().resource::<ResourceLedger>();
    assert!(
        ledger.is_empty(),
        "V1 build must not touch the resource ledger"
    );
    assert_eq!(
        ledger.total(ResourceKind::Minerals),
        0,
        "V1 build must not deposit any minerals"
    );
    // No Stockpile other than the completed one should exist
    // (the build is the only source of material flow, and it
    // does not move material in v1).
    let world = app.world_mut();
    let mut q = world.query::<&Stockpile>();
    let mut stockpiles = q.iter(world);
    let stockpile_count = stockpiles.by_ref().count();
    assert_eq!(
        stockpile_count, 1,
        "exactly one completed Stockpile must exist after the build"
    );
}

#[test]
fn idle_worker_in_build_cell_with_planned_idles_when_far() {
    // A worker placed far from the planned structure is
    // claimed, walks toward it, and only when it arrives does
    // progress start. This pins the "worker time advances"
    // contract: the planned structure is NOT decremented
    // during travel.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    // Place the worker one cell away so it has to walk.
    let far_pos = center + Vec2::new(ZONE_BLOCK_SIZE * 2.0, 0.0);
    let _worker = common::spawn_worker_at(&mut app, far_pos);

    // 1 tick for claim. The worker is now moving but has not
    // arrived. The planned structure's work_remaining must
    // still equal the full budget.
    app.update();
    let world = app.world();
    let planned_state = world.entity(planned).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned_state.work_remaining, DEFAULT_PLANNED_WORK_TICKS,
        "work_remaining must not decrease before the worker arrives"
    );
}

#[test]
fn planned_structure_is_owned_by_swarm_when_one_exists() {
    // Acceptance: "Planned Structures have a kind, owner,
    // location, build work remaining, and optional active
    // Worker reservation." The owner is the existing
    // [`OwnerSwarm`] component (per the project's
    // ownership pattern). When a swarm exists in the world
    // the auto-creation system must stamp the planned
    // structure with the swarm's `OwnerSwarm`, and the
    // completed Source Stockpile must keep the same
    // ownership after promotion.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, center);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &OwnerSwarm)>();
    let (planned, owner) = q
        .iter(world)
        .next()
        .expect("PlannedStructure must exist when a swarm is present");
    assert_eq!(
        owner.0, swarm,
        "planned structure must be owned by the swarm"
    );
    assert_eq!(planned.cell, cell);

    // Drive the build to completion and re-check ownership.
    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }
    let world = app.world_mut();
    let owner_after = world
        .query::<&OwnerSwarm>()
        .iter(world)
        .next()
        .expect("OwnerSwarm must be preserved on the completed structure");
    assert_eq!(
        owner_after.0, swarm,
        "completed structure keeps the swarm's ownership"
    );
}

#[test]
fn planned_structure_is_unowned_when_no_swarm_exists() {
    // When no swarm exists, the planned structure must
    // still emerge (the auto-creation is a swarm-agnostic
    // demand system). It just has no [`OwnerSwarm`] marker,
    // matching the unowned-paint contract in the rest of
    // the simulation.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(Entity, &PlannedStructure)>();
    let (planned_entity, _planned) = q
        .iter(world)
        .next()
        .expect("PlannedStructure must spawn even with no swarm");
    let has_owner = world.entity(planned_entity).get::<OwnerSwarm>().is_some();
    assert!(
        !has_owner,
        "planned structure must be unowned when no swarm exists"
    );
    // No Swarm entity exists in the world either.
    let swarm_count = world.query::<&Swarm>().iter(world).count();
    assert_eq!(swarm_count, 0);
}

#[test]
fn planned_structure_visual_flip_is_observable_via_sprite_color() {
    // Acceptance: "Planned Structures are visibly distinct
    // from completed structures." The Sprite color is the
    // visual hook the test pins. The planned color and the
    // completed color must differ, and the same entity must
    // transition from one to the other on completion.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned = common::spawn_planned_structure_at_cell(&mut app, cell);
    let _worker = common::spawn_worker_at(&mut app, center);

    // Capture the planned visual before the build runs.
    app.update();
    let pre_color = app
        .world()
        .entity(planned)
        .get::<Sprite>()
        .map(|s| s.color)
        .expect("PlannedStructure must carry a Sprite for the visual");
    assert_eq!(pre_color, planned_visual_color());

    // Drive the build to completion.
    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }
    let post_color = app
        .world()
        .entity(planned)
        .get::<Sprite>()
        .map(|s| s.color)
        .expect("completed structure must still carry a Sprite");
    assert_eq!(post_color, completed_visual_color());
    assert_ne!(
        pre_color, post_color,
        "the visual must flip from planned to completed on promotion"
    );
}
