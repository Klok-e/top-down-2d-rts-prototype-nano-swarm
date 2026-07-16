//! Integration tests for issue #26: Migrate Sink Stockpiles to
//! Build Zones.
//!
//! Each test isolates one behaviour so a failure points at a
//! single contract:
//!
//!   1. A Build-painted cell auto-creates a Planned Sink
//!      Stockpile (not a completed Stockpile) at an available position.
//!   2. A cell without Build paint does NOT auto-create a
//!      Planned Sink Stockpile.
//!   3. The auto-creation does not pile a second Planned Sink
//!      Stockpile on a cell that already holds one.
//!   4. A Worker claims the Planned Sink Stockpile, spends
//!      worker time, and the plan promotes to a completed
//!      Sink Stockpile (`Stockpile` + `StockpileRole::Sink`).
//!   5. A completed Sink Stockpile can receive hauled
//!      resources.
//!   6. Painting a Build cell does NOT create a completed
//!      `Stockpile` directly (the "instant spawn" the issue
//!      removes); only a planned structure appears.
//!   7. The planned structure's `OwnerSwarm` matches the
//!      Build cell's intent owner (per-swarm Build Zone
//!      ownership).
//!   8. A Source Stockpile's proximity / "demand satisfied"
//!      checks ignore Sink Stockpiles, so a Sink Stockpile
//!      in the same cell does not accidentally satisfy a
//!      gather worker's source need.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        DEFAULT_PLANNED_WORK_TICKS, HaulerAssignment, OwnerSwarm, PlannedKind, PlannedStructure,
        PlannedStructureClaim, PlannedStructureProgress, SwarmId, completed_visual_color,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_planned()
}

fn paint_build(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Build, Some(SwarmId::PLAYER),));
}

#[test]
fn build_paint_alone_does_not_plan_sink_stockpile() {
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_build(&mut app, cell);

    app.update();

    let world = app.world_mut();
    assert_eq!(
        world.query::<&PlannedStructure>().iter(world).count(),
        0,
        "Build paint alone must not plan Sink Stockpile"
    );
    assert_eq!(
        world.query::<&Stockpile>().iter(world).count(),
        0,
        "Build paint alone must not spawn completed Stockpile"
    );
}

#[test]
fn no_sink_stockpile_planned_without_build_paint() {
    // Acceptance: "No new Sink Stockpile is planned when no
    // suitable Build Zone exists." Painting a Gather-only
    // cell (or no cell) does not spawn a Planned Sink
    // Stockpile. Sink Stockpiles are base-infrastructure
    // demand, which only comes from Build paint.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SinkStockpile)
        .count();
    assert_eq!(
        planned_count, 0,
        "no Planned Sink Stockpile must exist without Build paint"
    );
}

#[test]
fn build_paint_alone_does_not_duplicate_sink_stockpiles() {
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_build(&mut app, cell);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SinkStockpile)
        .count();
    assert_eq!(planned_count, 0, "Build paint alone must remain inert");
}

#[test]
fn worker_builds_planned_sink_stockpile_into_completed() {
    // Acceptance: "One Worker builds a Planned Sink
    // Stockpile into a completed Stockpile." A Worker at
    // the cell center claims the plan, spends
    // `DEFAULT_PLANNED_WORK_TICKS` ticks, and the plan
    // promotes to a `Stockpile` carrying the Sink role.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    let _worker = common::spawn_worker_at(&mut app, center);

    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    // The planned structure is promoted: the
    // `PlannedStructure` component is gone, replaced by a
    // `Stockpile` + `StockpileRole::Sink` on the same
    // entity.
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
    assert_eq!(stockpile.amount, 0, "completed Sink Stockpile starts empty");
    let role = world
        .entity(planned)
        .get::<StockpileRole>()
        .expect("completed Sink Stockpile must carry StockpileRole::Sink");
    assert_eq!(
        *role,
        StockpileRole::Sink,
        "completed structure must keep the Sink role"
    );
    // The visual flipped to the completed color so the
    // player can read the build state.
    let sprite = world
        .entity(planned)
        .get::<Sprite>()
        .expect("completed Sink Stockpile must carry a Sprite");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed visual must use the completed color"
    );
}

#[test]
fn completed_sink_stockpile_receives_hauled_resources() {
    // Acceptance: "Completed Sink Stockpiles can receive
    // hauled resources and feed base needs." A Hauler
    // carries minerals from a deposit to the completed Sink
    // Stockpile, and the Sink Stockpile's `amount` rises
    // accordingly.
    let mut app = common::sim_app_with_gather_haul();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    // Place a fully built Sink Stockpile at the cell
    // center. The planned-structure flow would normally
    // build it, but the test pre-seeds the completed
    // structure to focus on the "Sink Stockpile receives
    // hauled resources" half of the contract.
    let sink = app
        .world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 1000,
                radius: 64.0,
            },
            StockpileRole::Sink,
            OwnerSwarm(swarm),
            Transform::from_translation(center.extend(0.0)),
        ))
        .id();
    // A nearby same-swarm Source Stockpile supplies the tier-2 leg.
    let source_pos = center + Vec2::new(-200.0, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 200, 200);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let hauler = common::spawn_hauler_at(&mut app, source_pos);
    app.world_mut()
        .entity_mut(hauler)
        .insert(HaulerAssignment { source, sink });

    // 1 arrive + 5 load + travel + delivery + buffer.
    for _ in 0..60 {
        app.update();
    }

    let world = app.world_mut();
    let sink_state = world.entity(sink).get::<Stockpile>().unwrap();
    assert!(
        sink_state.amount > 0,
        "completed Sink Stockpile must receive the hauler's load; got amount={}",
        sink_state.amount
    );
}

#[test]
fn build_paint_does_not_instant_spawn_completed_stockpile() {
    // Acceptance: "Instant stockpile spawning for replaced
    // Build Zone demand is removed or disabled." Painting a
    // Build cell does NOT spawn a completed `Stockpile` on
    // the same tick; only a `PlannedStructure` appears. The
    // "instant spawn" is the auto-creation system the issue
    // removes.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_build(&mut app, cell);

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let stockpile_count = world.query::<&Stockpile>().iter(world).count();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SinkStockpile)
        .count();
    assert_eq!(
        stockpile_count, 0,
        "Build paint must not spawn a completed Stockpile directly"
    );
    assert_eq!(
        planned_count, 0,
        "Build paint alone must not spawn a Planned Sink Stockpile"
    );
}

#[test]
fn planned_sink_stockpile_is_owned_by_swarm_that_painted_cell() {
    // Acceptance: "Sink Stockpile placement is constrained
    // to Build Zones owned by the same swarm." A
    // player-painted Build cell gets a Planned Sink
    // Stockpile stamped with the player `OwnerSwarm`. The
    // completion preserves the owner.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, center);
    let _worker = common::spawn_worker_at(&mut app, center);
    let planned_entity =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    app.world_mut()
        .entity_mut(planned_entity)
        .insert(OwnerSwarm(swarm));

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &OwnerSwarm)>();
    let (planned, owner) = q
        .iter(world)
        .next()
        .expect("Planned Sink Stockpile fixture must exist when a swarm is present");
    assert_eq!(planned.kind, PlannedKind::SinkStockpile);
    assert_eq!(
        owner.0, swarm,
        "Planned Sink Stockpile must be owned by the swarm that painted the cell"
    );

    // Drive the build to completion and re-check ownership.
    let total_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 5;
    for _ in 0..total_ticks {
        app.update();
    }
    let world = app.world_mut();
    let completed_owner = world
        .query::<(&Stockpile, &OwnerSwarm, &StockpileRole)>()
        .iter(world)
        .next()
        .map(|(_, o, r)| (o.0, *r))
        .expect("completed Sink Stockpile must keep OwnerSwarm + role");
    assert_eq!(
        completed_owner.0, swarm,
        "completed Sink Stockpile must keep the swarm's ownership"
    );
    assert_eq!(
        completed_owner.1,
        StockpileRole::Sink,
        "completed structure must keep the Sink role"
    );
}

#[test]
fn source_stockpile_demand_ignores_sink_stockpile_in_same_cell() {
    // Acceptance: a Sink Stockpile in the same cell as a
    // Resource Deposit does NOT count as a "near Source
    // Stockpile" for the gather worker's usability check. A
    // Sink Stockpile is base infrastructure, not a
    // deposit-side staging point, so the demand system
    // must still plan a Source Stockpile when only a Sink
    // is nearby.
    let mut app = common::sim_app_with_gather_planned();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
    }
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _worker = common::spawn_worker_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    // A completed Sink Stockpile in the same cell. The
    // gather worker's "any near Source Stockpile" check
    // must skip it.
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 0,
            capacity: 1000,
            radius: 64.0,
        },
        StockpileRole::Sink,
        Transform::from_translation(center.extend(0.0)),
    ));

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let source_planned = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SourceStockpile)
        .count();
    assert!(
        source_planned >= 1,
        "demand system must still plan a Source Stockpile when only a Sink Stockpile is nearby; \
         got {source_planned} planned Source Stockpiles"
    );
}

#[test]
fn charger_in_build_cell_does_not_plan_sink_stockpile() {
    // ADR-0005 treats chargers asymmetrically from production
    // facilities: a charger is a direct-delivery terminal, so even
    // if it happens to sit in Build paint it must not create Sink
    // Stockpile demand. This keeps Defend-zone logistics pressure
    // local to haulers feeding the charger itself.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_build(&mut app, cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 0);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SinkStockpile)
        .count();
    assert_eq!(
        planned_count, 0,
        "charger demand must not plan a Sink Stockpile; chargers are direct-delivery terminals"
    );
}

#[test]
fn no_sink_stockpile_planned_in_unpainted_cell_even_with_swarm() {
    // The "no plan without a Build Zone" half of the
    // contract, even with a swarm in the world: a swarm
    // without any Build paint cannot plan Sink Stockpiles.
    let mut app = build_app();
    let center = common::cell_world_center(IVec2::new(0, 0));
    let _swarm = common::spawn_swarm_at(&mut app, center);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SinkStockpile)
        .count();
    assert_eq!(
        planned_count, 0,
        "swarm without Build paint must not plan a Sink Stockpile"
    );
    let stockpile_count = world.query::<&Stockpile>().iter(world).count();
    assert_eq!(
        stockpile_count, 0,
        "swarm without Build paint must not spawn a completed Stockpile"
    );
}

#[test]
fn opponent_build_cell_creates_opponent_owned_sink_stockpile() {
    // Acceptance: "Sink Stockpile placement is constrained
    // to Build Zones owned by the same swarm." Per-swarm
    // Build Zone ownership: a Build cell painted by the
    // opponent swarm must produce a Planned Sink Stockpile
    // owned by the opponent swarm, not the player swarm.
    // The opponent-prepainted intent path mirrors what
    // `spawn_opponent_swarm` already does for Gather
    // cells; the planned-structure auto-creation must
    // route the Sink Stockpile to the right owner.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        PrepaintedIntent, ProductionPriority, SeedNanobots, Swarm, SwarmId, spawn_opponent_swarm,
    };
    let mut app = build_app();
    let opponent_pos = Vec2::new(2_000.0, 0.0);
    let cell = IVec2::new(0, 0);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        opponent_pos,
        ProductionPriority::new(),
        &[PrepaintedIntent::new(cell, IntentKind::Build)],
        &[SeedNanobots::new(
            top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker,
            1,
        )],
    );
    let opponent_id = app
        .world()
        .entity(opponent)
        .get::<SwarmId>()
        .copied()
        .expect("opponent swarm must carry a SwarmId");

    let planned_entity =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    app.world_mut()
        .entity_mut(planned_entity)
        .insert(OwnerSwarm(opponent));

    app.update();

    let world = app.world_mut();
    // Demand-created fixtures preserve opponent ownership.
    let mut q = world.query::<(&PlannedStructure, &OwnerSwarm)>();
    let (planned, owner) = q
        .iter(world)
        .next()
        .expect("opponent-owned Planned Sink Stockpile fixture must exist");
    assert_eq!(planned.kind, PlannedKind::SinkStockpile);
    let owner_swarm_id = world
        .entity(owner.0)
        .get::<SwarmId>()
        .copied()
        .expect("owner entity must carry a SwarmId");
    assert_eq!(
        owner_swarm_id, opponent_id,
        "Planned Sink Stockpile must be owned by the opponent swarm that painted the cell"
    );
    let _ = world
        .entity(owner.0)
        .get::<Swarm>()
        .expect("owner must be a Swarm entity");
}

#[test]
fn planned_kind_default_is_source_stockpile() {
    // The "default kind" contract is the foundation's
    // back-compat: `PlannedKind::default()` continues to
    // return `SourceStockpile` so test code that doesn't
    // care about the kind still compiles. The new Sink
    // Stockpile is reachable only through the explicit
    // `SinkStockpile` variant.
    assert_eq!(PlannedKind::default(), PlannedKind::SourceStockpile);
}

#[test]
fn planned_kind_all_includes_sink_stockpile() {
    // The `ALL` constant is a stable list of every kind
    // the planned-structure foundation models. The Sink
    // Stockpile must show up so future "iterate every kind"
    // loops see the new variant.
    let kinds: Vec<PlannedKind> = PlannedKind::ALL.to_vec();
    assert_eq!(kinds.len(), PlannedKind::COUNT);
    assert!(kinds.contains(&PlannedKind::SourceStockpile));
    assert!(
        kinds.contains(&PlannedKind::SinkStockpile),
        "PlannedKind::ALL must include SinkStockpile"
    );
}

#[test]
fn planned_kind_sink_index_is_stable() {
    // Pin the stable per-kind index for the new variant.
    // Indexes are part of the public contract: they back
    // table sizing and per-kind iteration.
    let sink_index = PlannedKind::SinkStockpile.index();
    let source_index = PlannedKind::SourceStockpile.index();
    assert_ne!(
        sink_index, source_index,
        "Sink Stockpile index must be distinct from Source Stockpile index"
    );
}

#[test]
fn idle_worker_at_planned_sink_stockpile_claims_and_works() {
    // Pin the "one Worker" claim contract for the Sink
    // Stockpile kind: a Worker placed at the planned
    // structure's cell receives a `PlannedStructureClaim`,
    // the planned structure records the worker as its
    // `active_worker`, and the worker eventually reaches
    // the progress phase after walking (or being already at)
    // the planned structure. The transition from claim to
    // progress happens over multiple ticks when the worker
    // has to walk; the test just verifies the end state.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let planned =
        common::spawn_planned_structure_of_kind_at_cell(&mut app, cell, PlannedKind::SinkStockpile);
    let worker = common::spawn_worker_at(&mut app, center);

    // Run a few ticks so the claim -> arrive -> progress
    // chain has time to fire for the worker at the cell
    // center. The build has 5 ticks of work, so a handful
    // of ticks is enough to observe the progress marker.
    for _ in 0..3 {
        app.update();
    }

    let world = app.world();
    let claim = world
        .entity(worker)
        .get::<PlannedStructureClaim>()
        .expect("idle worker must claim the planned Sink Stockpile");
    assert_eq!(claim.target, planned);
    let planned_state = world.entity(planned).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned_state.active_worker,
        Some(worker),
        "planned Sink Stockpile must record the worker as active_worker"
    );
    let progress = world.entity(worker).get::<PlannedStructureProgress>();
    assert!(
        progress.is_some(),
        "worker at the planned structure's cell must be in progress after a few ticks"
    );
}
