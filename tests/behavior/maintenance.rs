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
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, Charge, ChargerAssignment, DEGRADATION_INTERVAL_TICKS, DirectMovementComponent,
        LOW_CHARGE_THRESHOLD, MAINTENANCE_BUFFER_TICKS, MAINTENANCE_NEEDS_THRESHOLD,
        MAINTENANCE_WORK_DURATION_TICKS, MaintenanceAssignment, MaintenancePlugin,
        MaintenanceProgress, NanobotBundle, OwnerSwarm, ReturningToStockpile, STRUCTURE_MAX_HEALTH,
        SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind, SwarmId,
        worker_gather_delivery_system,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
};

#[path = "../common/mod.rs"]
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
fn real_stockpile_enters_shared_maintenance_lifecycle() {
    let mut app = build_app();
    let stockpile = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 100);

    app.update();

    let condition = app
        .world()
        .entity(stockpile)
        .get::<Structure>()
        .expect("real Stockpile must receive shared structure condition");
    assert_eq!(condition.health, STRUCTURE_MAX_HEALTH);
    assert_eq!(condition.ticks_since_maintained, 1);
}

#[test]
fn structure_degrades_when_no_workers_maintain_it() {
    // Acceptance: "Structures degrade when not maintained."
    // Once `ticks_since_maintained` exceeds the buffer, the structure loses
    // health at the fixed degradation cadence.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let structure = common::spawn_structure_at(&mut app, center);

    // Run past the buffer plus a few extra ticks so the
    // degradation actually kicks in.
    let ticks = (MAINTENANCE_BUFFER_TICKS + DEGRADATION_INTERVAL_TICKS + 1) as usize;
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
    let ticks =
        (MAINTENANCE_BUFFER_TICKS + STRUCTURE_MAX_HEALTH * DEGRADATION_INTERVAL_TICKS + 1) as usize;
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
        .paint(cell, IntentKind::Build);
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
        .paint(cell, IntentKind::Build);
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
        .paint(cell, IntentKind::Build);
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
        .paint(cell, IntentKind::Build);
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
fn real_structure_requests_maintenance_without_build_paint() {
    let mut app = build_app();
    let center = common::cell_world_center(IVec2::ZERO);
    let stockpile = common::spawn_stockpile(&mut app, center, 0, 100);
    app.update();
    app.world_mut()
        .entity_mut(stockpile)
        .get_mut::<Structure>()
        .expect("stockpile has shared condition")
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let worker = app.world().entity(worker);
    assert!(
        worker.get::<MaintenanceAssignment>().is_some()
            || worker.get::<MaintenanceProgress>().is_some(),
        "maintenance originates from the real structure, not Build paint",
    );
}

#[test]
fn unattended_valid_charger_rejects_worker_upkeep_and_degrades() {
    let mut app = common::sim_app_with_charge_planned();
    app.add_plugins(MaintenancePlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, center);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let charger = common::spawn_operational_charger_at(&mut app, cell, 20);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    app.world_mut()
        .entity_mut(charger)
        .get_mut::<Structure>()
        .expect("Charger has shared condition")
        .ticks_since_maintained = MAINTENANCE_BUFFER_TICKS + DEGRADATION_INTERVAL_TICKS - 1;
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    assert!(
        world
            .entity(worker)
            .get::<MaintenanceAssignment>()
            .is_none()
            && world.entity(worker).get::<MaintenanceProgress>().is_none(),
        "unattended Charger capacity must not consume Worker upkeep",
    );
    assert_eq!(
        world
            .entity(charger)
            .get::<Structure>()
            .expect("unattended Charger remains a structure")
            .health,
        STRUCTURE_MAX_HEALTH - 1,
        "unattended valid Charger must degrade through shared condition rules",
    );
}

#[test]
fn erased_defend_paint_leaves_charger_inactive_and_degrading() {
    let mut app = common::sim_app_with_charge_planned();
    app.add_plugins(MaintenancePlugin);
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, center);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let charger = common::spawn_operational_charger_at(&mut app, cell, 20);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    let defender = common::spawn_defender_at(&mut app, center);

    app.update();

    assert!(
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .remove(cell, IntentKind::Defend),
    );
    {
        let mut charger_entity = app.world_mut().entity_mut(charger);
        let mut condition = charger_entity
            .get_mut::<Structure>()
            .expect("Charger has shared condition");
        condition.ticks_since_maintained =
            MAINTENANCE_BUFFER_TICKS + DEGRADATION_INTERVAL_TICKS - 1;
    }
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    let worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    assert!(
        world.get_entity(charger).is_ok(),
        "paint erasure must not despawn the completed Charger",
    );
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "a Charger outside owned Defend paint must be inactive for service",
    );
    assert!(
        world
            .entity(worker)
            .get::<MaintenanceAssignment>()
            .is_none()
            && world.entity(worker).get::<MaintenanceProgress>().is_none(),
        "a Charger outside owned Defend paint must not receive Worker upkeep",
    );
    assert_eq!(
        world
            .entity(charger)
            .get::<Structure>()
            .expect("Charger remains a structure")
            .health,
        STRUCTURE_MAX_HEALTH - 1,
        "unmaintained Charger must degrade through the existing condition rules",
    );
}

#[test]
fn en_route_service_assigns_worker_to_stale_charger() {
    let mut app = common::sim_app_with_charge_planned();
    app.add_plugins(MaintenancePlugin);
    let charger_cell = IVec2::ZERO;
    let charger_center = common::cell_world_center(charger_cell);
    let swarm = common::spawn_swarm_at(&mut app, charger_center);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        charger_cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let charger = common::spawn_operational_charger_at(&mut app, charger_cell, 20);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));
    app.world_mut()
        .entity_mut(charger)
        .get_mut::<Structure>()
        .expect("Charger has shared condition")
        .ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(IVec2::new(3, 0)));
    app.world_mut().entity_mut(defender).insert((
        ChargerAssignment { charger },
        DirectMovementComponent {
            xy: charger_center,
            stop_radius: 32.0,
        },
    ));
    let worker = common::spawn_worker_at(&mut app, charger_center);

    app.update();

    let worker = app.world().entity(worker);
    let target = worker
        .get::<MaintenanceAssignment>()
        .map(|assignment| assignment.target)
        .or_else(|| {
            worker
                .get::<MaintenanceProgress>()
                .map(|progress| progress.target)
        });
    assert_eq!(
        target,
        Some(charger),
        "active Charger service must produce assignable Worker upkeep",
    );
}

#[test]
fn degraded_stockpile_rejects_worker_delivery_without_losing_cargo() {
    let mut app = App::new();
    app.add_systems(Update, worker_gather_delivery_system);
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    let stockpile = app
        .world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 100,
                radius: 32.0,
            },
            condition,
            Transform::default(),
        ))
        .id();
    let worker = app
        .world_mut()
        .spawn((
            NanobotBundle::default(),
            Transform::default(),
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 4,
            },
            ReturningToStockpile { stockpile },
        ))
        .id();

    app.update();

    assert_eq!(
        app.world()
            .entity(stockpile)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        0,
    );
    assert_eq!(app.world().entity(worker).get::<Cargo>().unwrap().amount, 4);
    assert!(
        app.world()
            .entity(worker)
            .get::<ReturningToStockpile>()
            .is_none(),
        "worker must release degraded destination and retry later",
    );
}
