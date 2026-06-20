//! Integration tests for issue #12: Structure maintenance and
//! degradation.
//!
//! Each test isolates one behaviour so a failure points at a single
//! contract: maintenance-state tracking, degradation, collapse,
//! worker assignment, no-resource consumption, and stable
//! maintenance under enough worker time. The pure-helper unit
//! tests for the data layer live in `src/nanobot/maintenance.rs`.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        MaintenanceAssignment, MaintenanceProgress, Structure, StructureKind,
        MAINTENANCE_BUFFER_TICKS, MAINTENANCE_NEEDS_THRESHOLD, MAINTENANCE_WORK_DURATION_TICKS,
        STRUCTURE_MAX_HEALTH,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
};

mod common;

fn build_app() -> App {
    common::sim_app_with_maintenance()
}

fn structure_buffer(app: &App, structure: Entity) -> u32 {
    app.world()
        .entity(structure)
        .get::<Structure>()
        .map(|s| s.ticks_since_maintained)
        .unwrap_or(u32::MAX)
}

fn structure_health(app: &App, structure: Entity) -> Option<u32> {
    app.world()
        .entity(structure)
        .get::<Structure>()
        .map(|s| s.health)
}

#[test]
fn structure_tracks_maintenance_state_via_buffer_counter() {
    // Acceptance: "Structures track maintenance state/health."
    // The `Structure` component carries `ticks_since_maintained`
    // and increments it every tick when no worker is maintaining.
    // Within the maintenance buffer, the structure's health does
    // not change.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);

    for _ in 0..5 {
        app.update();
    }

    let s = app
        .world()
        .entity(structure)
        .get::<Structure>()
        .expect("structure must still exist within the buffer");
    assert_eq!(
        s.ticks_since_maintained, 5,
        "buffer counter must increment each tick"
    );
    assert_eq!(
        s.health, STRUCTURE_MAX_HEALTH,
        "no degradation within the buffer; got {}",
        s.health
    );
}

#[test]
fn structure_degrades_when_no_workers_maintain_it() {
    // Acceptance: "Structures degrade when not maintained."
    // Once `ticks_since_maintained` exceeds the buffer, the
    // structure loses one health per tick.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);

    // Run past the buffer plus a few extra ticks so the
    // degradation actually kicks in.
    let ticks = (MAINTENANCE_BUFFER_TICKS + 5) as usize;
    for _ in 0..ticks {
        app.update();
    }

    let health =
        structure_health(&app, structure).expect("structure must still exist mid-degradation");
    assert!(
        health < STRUCTURE_MAX_HEALTH,
        "structure must have lost health after the buffer expired; got {}",
        health
    );
    // Sanity: the loss is bounded by the number of ticks past
    // the buffer.
    let expected_max_loss = (ticks as u32) - MAINTENANCE_BUFFER_TICKS;
    assert!(
        STRUCTURE_MAX_HEALTH - health <= expected_max_loss,
        "degradation must not exceed the elapsed-ticks budget; lost {} expected at most {}",
        STRUCTURE_MAX_HEALTH - health,
        expected_max_loss
    );
}

#[test]
fn structure_collapses_at_zero_health() {
    // Acceptance: "...structures...may collapse" (issue body).
    // A structure with no workers and enough elapsed ticks must
    // be despawned when its health reaches zero.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);

    // Run past the buffer plus the full health bar plus a few
    // extra ticks so the structure is guaranteed to have
    // collapsed.
    let ticks = (MAINTENANCE_BUFFER_TICKS + STRUCTURE_MAX_HEALTH + 5) as usize;
    for _ in 0..ticks {
        app.update();
    }

    assert!(
        app.world().get_entity(structure).is_err(),
        "structure must be despawned when it collapses"
    );
}

#[test]
fn worker_travels_to_and_maintains_stale_structure() {
    // Acceptance: "Workers can spend time maintaining structures."
    // A worker in a Build-painted cell with a stale structure
    // must be assigned to the structure and reset its buffer
    // counter. After enough ticks the structure's buffer must
    // sit below the threshold and the structure must be at full
    // health.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);
    // Make the structure stale so the maintenance system
    // immediately has a target on the first tick.
    app.world_mut()
        .entity_mut(structure)
        .get_mut::<Structure>()
        .unwrap()
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    let _worker = common::spawn_worker_at(&mut app, center);

    // Run long enough for one full maintenance cycle plus a
    // buffer to elapse, so we can be sure the worker is not
    // simply lucky on a single shift.
    let ticks = (MAINTENANCE_WORK_DURATION_TICKS as usize) + 30;
    for _ in 0..ticks {
        app.update();
    }

    let buffer = structure_buffer(&app, structure);
    let health = structure_health(&app, structure)
        .expect("structure must still exist with a worker present");
    assert!(
        buffer < MAINTENANCE_NEEDS_THRESHOLD,
        "buffer must be reset by maintenance; got {}",
        buffer
    );
    assert_eq!(
        health, STRUCTURE_MAX_HEALTH,
        "structure must be at full health under worker maintenance; got {}",
        health
    );
}

#[test]
fn maintenance_does_not_consume_stockpile_resources() {
    // Acceptance: "Maintenance consumes Worker time only, not
    // extra resources." A worker maintaining a structure must
    // not pull from a local stockpile or move resources through
    // the ledger. The stockpile amount and ledger total must be
    // unchanged after a maintenance cycle.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);
    app.world_mut()
        .entity_mut(structure)
        .get_mut::<Structure>()
        .unwrap()
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    // Stockpile in the same cell with material that must NOT
    // be drained by the maintenance work.
    let stockpile = common::spawn_stockpile(&mut app, center, 100, 1000);
    common::spawn_worker_at(&mut app, center);

    // Run long enough to cover at least one full maintenance
    // shift plus the worker's return visit. Any pull from the
    // stockpile would be visible in the amount.
    let ticks = 50;
    for _ in 0..ticks {
        app.update();
    }

    let s = app
        .world()
        .entity(stockpile)
        .get::<Stockpile>()
        .expect("stockpile must still exist");
    assert_eq!(
        s.amount, 100,
        "maintenance must not consume stockpile material; got {}",
        s.amount
    );
    // The ledger is updated by the resource movement systems
    // (gather/extract, delivery). Spawning a stockpile directly
    // does not touch the ledger; the only way for the ledger
    // total to change during this test is for the maintenance
    // work system to call `ledger.add` or `ledger.remove`. It
    // does neither, so the ledger must still be empty.
    let ledger = app.world().resource::<ResourceLedger>();
    assert_eq!(
        ledger.total(ResourceKind::Minerals),
        0,
        "maintenance must not touch the resource ledger; got {}",
        ledger.total(ResourceKind::Minerals)
    );
}

#[test]
fn sufficient_worker_time_keeps_structure_stable() {
    // Acceptance: "Sufficient Worker maintenance stabilizes
    // structures." A single worker cycling through maintenance
    // shifts must keep the structure at full health. The buffer
    // counter must never reach the unstable regime.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);
    common::spawn_worker_at(&mut app, center);

    // Run long enough for several maintenance cycles. The cycle
    // is `MAINTENANCE_WORK_DURATION_TICKS` work + a few idle
    // ticks before the next shift, so 200 ticks covers many
    // cycles.
    for _ in 0..200 {
        app.update();
    }

    let s = app
        .world()
        .entity(structure)
        .get::<Structure>()
        .expect("structure must still exist with a worker present");
    assert_eq!(
        s.health, STRUCTURE_MAX_HEALTH,
        "structure must stay at full health under continuous maintenance; got {}",
        s.health
    );
    assert!(
        s.ticks_since_maintained < MAINTENANCE_BUFFER_TICKS,
        "buffer must never reach the unstable regime; got {}",
        s.ticks_since_maintained
    );
}

#[test]
fn idle_worker_picks_maintenance_over_idling_when_structure_is_stale() {
    // Acceptance: "Workers can spend time maintaining structures."
    // A worker that is idle in a Build cell with a stale
    // structure must receive a maintenance assignment, not just
    // stay idle. The worker's marker set after one tick is the
    // observable proof.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);
    app.world_mut()
        .entity_mut(structure)
        .get_mut::<Structure>()
        .unwrap()
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    let worker = common::spawn_worker_at(&mut app, center);

    // One tick is enough for the assignment system to fire
    // because the worker starts at the structure's position.
    app.update();

    let world = app.world();
    let has_marker = world
        .entity(worker)
        .get::<MaintenanceAssignment>()
        .is_some()
        || world.entity(worker).get::<MaintenanceProgress>().is_some();
    assert!(
        has_marker,
        "worker must receive a maintenance assignment when a stale structure is in the cell"
    );
}

#[test]
fn maintenance_is_skipped_when_worker_is_busy_with_build() {
    // Sanity check: a worker already doing build work in a
    // build cell must not also be assigned to maintenance in
    // the same tick. The maintenance system must filter out
    // workers with build markers, otherwise the two systems
    // would double-book the work slot.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, PAINT_STRENGTH_CAP);
    let center = common::cell_world_center(cell);
    // Both a build site and a stale structure exist in the
    // same cell. The build system wins, the maintenance system
    // must not also pick the worker.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::BuildSite;
    app.world_mut().spawn((
        BuildSite::new(cell, StructureKind::Basic),
        Transform::from_translation(center.extend(0.0)),
    ));
    let structure = common::spawn_structure_at(&mut app, center);
    app.world_mut()
        .entity_mut(structure)
        .get_mut::<Structure>()
        .unwrap()
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let has_build_marker = world
        .entity(worker)
        .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::BuildAssignment>()
        .is_some()
        || world
            .entity(worker)
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::BuildProgress>()
            .is_some();
    let has_maint_marker = world
        .entity(worker)
        .get::<MaintenanceAssignment>()
        .is_some()
        || world.entity(worker).get::<MaintenanceProgress>().is_some();
    assert!(
        has_build_marker,
        "build system should pick the build site first"
    );
    assert!(
        !has_maint_marker,
        "maintenance system must not double-book a worker who is already on build work"
    );
}
