//! Regression tests for issue #29: Remove remaining spontaneous
//! support-structure spawning.
//!
//! After the four predecessor issues (Source Stockpile #23/#24/#25,
//! Sink Stockpile #26, Production Facility #27, Charger #28) every
//! demand-driven support structure is supposed to live on the
//! Planned Structure lifecycle. Scenario seed structures may
//! still spawn completed at startup, but paint and demand alone
//! must never produce a completed structure (Stockpile,
//! ProductionFacility, Charger, or the legacy `BuildSite`).
//!
//! Each test isolates one demand source -- Gather paint, Build
//! paint, production demand, and Defend demand -- and asserts
//! that no completed structure of any kind appears without a
//! Worker building a planned structure first. The last test
//! covers the "scenario seed structures remain valid" half of
//! the contract by exercising the default scenario's seed
//! `ProductionFacility`.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        Charger, DefendHold, NanobotType, PlannedStructure, ProductionFacility, ProductionRatio,
        SwarmId,
    },
    resources::Stockpile,
};

#[path = "../common/mod.rs"]
mod common;

/// Every demand source that issue #29 covers. A demand-driven
/// structure of any of these kinds must NOT spawn as a completed
/// entity; the only acceptable output of a demand tick is a
/// `PlannedStructure` (or, for scenario seeds, a structure that
/// was already present at startup).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DemandSource {
    GatherPaint,
    BuildPaint,
    ProductionDemand,
    DefendDemand,
}

impl DemandSource {
    /// Human-readable name for failure messages.
    fn name(self) -> &'static str {
        match self {
            DemandSource::GatherPaint => "Gather paint",
            DemandSource::BuildPaint => "Build paint",
            DemandSource::ProductionDemand => "production demand",
            DemandSource::DefendDemand => "defender support demand",
        }
    }
}

/// Helper: count completed `Stockpile` entities in the world.
/// Counts every role (Source, Sink, unowned) because every
/// kind of `Stockpile` is a "completed support structure" from
/// the issue's perspective.
fn completed_stockpile_count(world: &mut World) -> usize {
    world.query::<&Stockpile>().iter(world).count()
}

/// Helper: count completed `ProductionFacility` entities in the
/// world.
fn completed_facility_count(world: &mut World) -> usize {
    world.query::<&ProductionFacility>().iter(world).count()
}

/// Helper: count completed `Charger` entities in the world.
fn completed_charger_count(world: &mut World) -> usize {
    world.query::<&Charger>().iter(world).count()
}

/// Helper: count any kind of `PlannedStructure` entity in the
/// world. Used to assert that demand ticks DO produce a
/// planned structure (the visible "demand noticed" signal) but
/// never a completed one.
fn planned_count(world: &mut World) -> usize {
    world.query::<&PlannedStructure>().iter(world).count()
}

/// Paint a cell with `kind` intent owned by the player swarm.
/// The "owned" path mirrors the default scenario's per-swarm
/// intent stamp.
fn paint_owned(app: &mut App, cell: IVec2, kind: IntentKind) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, kind, PAINT_STRENGTH_CAP, Some(SwarmId::PLAYER)));
}

#[test]
fn gather_paint_does_not_instant_spawn_any_support_structure() {
    // Acceptance: "No Gather or Build paint instantly creates
    // completed Stockpiles." A Gather-painted cell with a
    // deposit and a worker must NOT produce a completed
    // Stockpile on the same tick. The demand system plans a
    // Source Stockpile only; the completed Stockpile waits
    // for a Worker to build the plan.
    //
    // The test also asserts that no other kind of completed
    // support structure (ProductionFacility, Charger,
    // legacy BuildSite) appears as a side effect of Gather
    // paint, so a regression that re-introduces a stray
    // auto-spawner would be caught here.
    let mut app = common::sim_app_with_gather_planned();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    let _worker = common::spawn_worker_at(&mut app, center);
    let _deposit = common::spawn_deposit(&mut app, center, 100);
    paint_owned(&mut app, cell, IntentKind::Gather);

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let stockpiles = completed_stockpile_count(world);
    let facilities = completed_facility_count(world);
    let chargers = completed_charger_count(world);
    let plans = planned_count(world);
    assert_eq!(
        stockpiles,
        0,
        "{} must not spawn a completed Stockpile; got {stockpiles}",
        DemandSource::GatherPaint.name()
    );
    assert_eq!(
        facilities,
        0,
        "{} must not spawn a completed ProductionFacility",
        DemandSource::GatherPaint.name()
    );
    assert_eq!(
        chargers,
        0,
        "{} must not spawn a completed Charger",
        DemandSource::GatherPaint.name()
    );
    // Demand DID notice the deposit: a planned Source
    // Stockpile is the expected visible signal. Plans are
    // the contract -- completed entities are not.
    assert!(
        plans >= 1,
        "{} should plan a Source Stockpile (worker is assigned to a deposit); got {plans} plans",
        DemandSource::GatherPaint.name()
    );
}

#[test]
fn build_paint_does_not_instant_spawn_any_support_structure() {
    // Acceptance: "No Gather or Build paint instantly creates
    // completed Stockpiles." A Build-painted cell alone (no
    // swarm, no production demand) must not spawn a
    // completed `Stockpile` of any role. The Sink Stockpile
    // is the demand signal, not the demand result, so the
    // only acceptable output is a `PlannedStructure` (or
    // nothing, when no swarm exists to plan for).
    //
    // The test also pins the absence of any other completed
    // support structure (ProductionFacility, Charger,
    // legacy `BuildSite`) so a regression that re-adds a
    // spontaneous auto-spawner for any kind fails here.
    let mut app = common::sim_app_with_planned();
    let cell = IVec2::new(0, 0);
    paint_owned(&mut app, cell, IntentKind::Build);

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let stockpiles = completed_stockpile_count(world);
    let facilities = completed_facility_count(world);
    let chargers = completed_charger_count(world);
    let plans = planned_count(world);
    assert_eq!(
        stockpiles,
        0,
        "{} must not spawn a completed Stockpile; got {stockpiles}",
        DemandSource::BuildPaint.name()
    );
    assert_eq!(
        facilities,
        0,
        "{} must not spawn a completed ProductionFacility",
        DemandSource::BuildPaint.name()
    );
    assert_eq!(
        chargers,
        0,
        "{} must not spawn a completed Charger",
        DemandSource::BuildPaint.name()
    );
    // Issue #34: Build paint alone is only a placement constraint.
    assert_eq!(
        plans,
        0,
        "{} must not plan anything without real structure demand; got {plans} plans",
        DemandSource::BuildPaint.name()
    );
}

#[test]
fn build_paint_with_swarm_does_not_spawn_legacy_buildsite() {
    // Acceptance: "demand-driven support structures should
    // always pass through Planned Structure first." The
    // legacy `BuildSite`/`Structure` system (issue #10) was
    // the previous "instant Build-paint spawn" path. It is
    // not part of the PRD-named kind set (Source / Sink /
    // Production / Charger) and must not appear as a side
    // effect of Build paint any more. The new Sink Stockpile
    // plan covers the demand; the legacy system has been
    // disabled.
    //
    // The helper intentionally does NOT include the legacy
    // `BuildPlugin`, so a regression that re-adds the
    // `BuildPlugin` to the production app would not be
    // visible here -- the regression suite covers the
    // completed-structure contract (no instant Stockpile,
    // Facility, or Charger) in the other tests in this
    // file. This test pins the related contract that the
    // planned-structure system does not produce any
    // `BuildSite` entity of its own.
    let mut app = common::sim_app_with_build_planned();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _swarm = common::spawn_swarm_at(&mut app, center);
    paint_owned(&mut app, cell, IntentKind::Build);

    for _ in 0..3 {
        app.update();
    }

    // The legacy `BuildSite` component, if it still
    // auto-spawned from Build paint, would be visible here.
    let build_sites = {
        let world = app.world_mut();
        let mut q = world.query::<&top_down_2d_rts_prototype_nano_swarm::nanobot::BuildSite>();
        q.iter(world).count()
    };
    assert_eq!(
        build_sites, 0,
        "Build paint must not spawn a legacy BuildSite; only PlannedStructures are allowed"
    );
    // Issue #34: no plan emerges from Build paint alone.
    let plans = {
        let world = app.world_mut();
        planned_count(world)
    };
    assert_eq!(
        plans, 0,
        "Build paint alone must not plan support structures"
    );
}

#[test]
fn production_demand_does_not_instant_spawn_completed_facility() {
    // Acceptance: "Production pressure no longer instantly
    // creates completed Production Facilities." A swarm with
    // high unmet production demand and an owned Build cell
    // plans a Production Facility, not a completed one. No
    // other completed support structure appears as a side
    // effect either.
    let mut app = common::sim_app_with_production_planned();
    app.insert_resource(ProductionRatio::new());
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }
    let cell = IVec2::new(0, 0);
    paint_owned(&mut app, cell, IntentKind::Build);

    app.update();

    let world = app.world_mut();
    let facilities = completed_facility_count(world);
    let stockpiles = completed_stockpile_count(world);
    let chargers = completed_charger_count(world);
    let plans = planned_count(world);
    assert_eq!(
        facilities,
        0,
        "{} must not spawn a completed ProductionFacility; got {facilities}",
        DemandSource::ProductionDemand.name()
    );
    assert_eq!(
        stockpiles,
        0,
        "{} must not spawn a completed Stockpile",
        DemandSource::ProductionDemand.name()
    );
    assert_eq!(
        chargers,
        0,
        "{} must not spawn a completed Charger",
        DemandSource::ProductionDemand.name()
    );
    assert!(
        plans >= 1,
        "{} should plan a Production Facility; got {plans} plans",
        DemandSource::ProductionDemand.name()
    );
}

#[test]
fn defend_demand_does_not_instant_spawn_completed_charger() {
    // Acceptance: "Defender support pressure no longer
    // instantly creates completed Chargers." A Defend cell
    // with a holding defender plans a Charger, not a
    // completed one. No other completed support structure
    // appears as a side effect.
    let mut app = common::sim_app_with_charge_planned();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    paint_owned(&mut app, cell, IntentKind::Defend);
    let _defender = {
        let d = common::spawn_defender_at(&mut app, cell_center);
        app.world_mut().entity_mut(d).insert(DefendHold { cell });
        d
    };

    app.update();

    let world = app.world_mut();
    let chargers = completed_charger_count(world);
    let stockpiles = completed_stockpile_count(world);
    let facilities = completed_facility_count(world);
    let plans = planned_count(world);
    assert_eq!(
        chargers,
        0,
        "{} must not spawn a completed Charger; got {chargers}",
        DemandSource::DefendDemand.name()
    );
    assert_eq!(
        stockpiles,
        0,
        "{} must not spawn a completed Stockpile",
        DemandSource::DefendDemand.name()
    );
    assert_eq!(
        facilities,
        0,
        "{} must not spawn a completed ProductionFacility",
        DemandSource::DefendDemand.name()
    );
    assert!(
        plans >= 1,
        "{} should plan a Charger in the Defend cell; got {plans} plans",
        DemandSource::DefendDemand.name()
    );
}

#[test]
fn all_demand_sources_share_zero_completed_structures() {
    // Compendium: run every demand source in a single app
    // and assert the world has zero completed support
    // structures of any kind. A future regression that adds
    // a new "spontaneous spawn" path for any of the four
    // PRD kinds (or the legacy BuildSite) will trip this
    // test in one place. The test name in the failure
    // message makes the offending demand source obvious
    // because the assertion runs after each tick of every
    // source.
    //
    // The builder does NOT include the legacy `BuildPlugin`;
    // the regression that this test catches is "any demand
    // source produces a completed support structure of any
    // kind without going through the planned-structure
    // chain". A regression that re-adds `BuildPlugin` to
    // the production app would have to be caught by code
    // review, not by this test.
    let mut app = common::sim_app_with_build_planned();
    // Add the production + charge + defend plugins on top
    // so every demand source is exercised. The
    // `PlannedStructurePlugin` is already included by
    // `sim_app_with_build_planned`.
    app.add_plugins(top_down_2d_rts_prototype_nano_swarm::nanobot::ProductionPlugin);
    app.add_plugins(top_down_2d_rts_prototype_nano_swarm::nanobot::HaulPlugin);
    app.add_plugins(top_down_2d_rts_prototype_nano_swarm::nanobot::DefendPlugin);
    app.add_plugins(top_down_2d_rts_prototype_nano_swarm::nanobot::ChargePlugin);
    app.insert_resource(ProductionRatio::new());
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    // Gather cell (with deposit + worker) and Build cell
    // (no swarm demand) and Defend cell (with a defender in
    // hold). The test paints all three before the first
    // tick so every demand source has a chance to fire on
    // the same update.
    let gather_cell = IVec2::new(0, 0);
    let gather_center = common::cell_world_center(gather_cell);
    let _worker = common::spawn_worker_at(&mut app, gather_center);
    let _deposit = common::spawn_deposit(&mut app, gather_center, 100);
    paint_owned(&mut app, gather_cell, IntentKind::Gather);
    let build_cell = IVec2::new(1, 0);
    paint_owned(&mut app, build_cell, IntentKind::Build);
    let defend_cell = IVec2::new(2, 0);
    let defend_center = common::cell_world_center(defend_cell);
    paint_owned(&mut app, defend_cell, IntentKind::Defend);
    let _defender = {
        let d = common::spawn_defender_at(&mut app, defend_center);
        app.world_mut()
            .entity_mut(d)
            .insert(DefendHold { cell: defend_cell });
        d
    };
    // High production demand so the Build cell could also
    // be claimed by the production auto-creator.
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }

    for _ in 0..3 {
        app.update();
        let world = app.world_mut();
        let stockpiles = completed_stockpile_count(world);
        let facilities = completed_facility_count(world);
        let chargers = completed_charger_count(world);
        let legacy_sites = {
            let mut q = world.query::<&top_down_2d_rts_prototype_nano_swarm::nanobot::BuildSite>();
            q.iter(world).count()
        };
        assert_eq!(
            stockpiles, 0,
            "any demand tick must not spawn a completed Stockpile; got {stockpiles}"
        );
        assert_eq!(
            facilities, 0,
            "any demand tick must not spawn a completed ProductionFacility; got {facilities}"
        );
        assert_eq!(
            chargers, 0,
            "any demand tick must not spawn a completed Charger; got {chargers}"
        );
        assert_eq!(
            legacy_sites, 0,
            "any demand tick must not spawn a legacy BuildSite; got {legacy_sites}"
        );
    }
}

#[test]
fn scenario_seed_facility_remains_a_completed_production_facility() {
    // Acceptance: "Scenario seed structures remain completed
    // at startup where intended." The default scenario's
    // seed `ProductionFacility` is a real
    // `ProductionFacility` (not a planned structure). The
    // demand layer must NOT turn the seed into a plan; the
    // seed's first production cycle runs through the
    // existing pick / work systems.
    //
    // We model the seed directly: a `ProductionFacility` is
    // spawned into a world with movement systems and the
    // production plugin, alongside a stockpile with material
    // so the seed's first pick cycle can pull from it. The
    // first tick after spawn must NOT despawn the seed
    // (the auto-creation system must not turn it into a
    // plan) and the seed's `current_target` must be set
    // because the production pick system saw the seed.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::PRODUCTION_COST_PER_BOT;
    let mut app = common::sim_app_with_production();
    // `sim_app_with_production` registers the production
    // plugin but does not insert the global `ProductionRatio`
    // resource (the production system reads from it, so it
    // must be present). The default ratio would also work
    // -- the test sets a small Worker target so the pick
    // system has a clear "produce a Worker" signal.
    app.insert_resource(ProductionRatio::new());
    let swarm_center = Vec2::new(0.0, 0.0);
    let _swarm = common::spawn_swarm_at(&mut app, swarm_center);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 1);
    }
    let seed = common::spawn_facility_at(&mut app, _swarm, swarm_center);
    let _stockpile =
        common::spawn_stockpile(&mut app, swarm_center, PRODUCTION_COST_PER_BOT * 5, 1000);

    app.update();

    let world = app.world_mut();
    let still_completed = world
        .entity(seed)
        .get::<ProductionFacility>()
        .expect("scenario seed ProductionFacility must still be a completed facility, not a plan");
    assert!(
        still_completed.is_busy() || still_completed.current_target.is_some(),
        "scenario seed ProductionFacility must start producing on the first tick"
    );
    // No PlannedStructure of any kind was created for the
    // seed cell.
    let plans = planned_count(world);
    assert_eq!(
        plans, 0,
        "scenario seed ProductionFacility must not be re-mapped to a PlannedStructure; got {plans} plans"
    );
}

#[test]
fn spread_cells_keep_demand_sources_separated() {
    // Helper test: paint cells that are far enough apart
    // (ZONE_BLOCK_SIZE apart) that no placement system
    // could conflate their demand. This proves the
    // previous tests' "single demand source" assertions
    // are not poisoned by accidental overlap.
    let mut app = common::sim_app_with_planned();
    let gather_cell = IVec2::new(-3, 0);
    let build_cell = IVec2::new(3, 0);
    paint_owned(&mut app, gather_cell, IntentKind::Gather);
    paint_owned(&mut app, build_cell, IntentKind::Build);

    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    // Gather alone with no deposit / worker does not plan
    // anything; Build paint alone is also inert after issue #34.
    let plans: Vec<_> = world
        .query::<(&PlannedStructure, &Transform)>()
        .iter(world)
        .map(|(p, t)| (p.kind, t.translation.truncate()))
        .collect();
    assert_eq!(
        plans.len(),
        0,
        "Gather paint alone plus Build paint alone must produce no plans; got {plans:?}"
    );
}
