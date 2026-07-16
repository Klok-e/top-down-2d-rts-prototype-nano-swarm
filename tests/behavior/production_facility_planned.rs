//! Integration tests for issue #27: Migrate Production Facilities
//! to Planned Structures.
//!
//! Each test isolates one behaviour so a failure points at a
//! single contract:
//!
//!   1. Production pressure creates a Planned Production
//!      Facility instead of an instant completed facility.
//!   2. Planned Production Facility placement is constrained
//!      to an owned Build Zone.
//!   3. No new Production Facility is planned when no
//!      suitable Build Zone exists.
//!   4. One Worker builds the planned facility to completion.
//!   5. Completed Production Facilities consume resources
//!      and produce nanobots through existing production
//!      rules.
//!   6. Existing starting scenario facilities remain valid
//!      seed structures.
//!   7. The "demand does not pile plans in a single cell"
//!      half of the contract (the auto-creation system sees
//!      the planned cell as occupied).
//!   8. The planning target guides placement only; completion starts idle
//!      until logistics pays a full production cycle.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        DEFAULT_PLANNED_WORK_TICKS, NanobotType, OwnerSwarm, PlannedKind, PlannedProductionTarget,
        PlannedStructure, PlannedStructureClaim, PlannedStructureProgress, ProductionFacility,
        ProductionRatio, SwarmId, completed_visual_color, planned_visual_color,
    },
    resources::Stockpile,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    // Empty global ratio by default; each test sets the
    // ratio(s) it needs to drive demand.
    let mut app = common::sim_app_with_production_planned();
    app.insert_resource(ProductionRatio::new());
    app
}

fn paint_build(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Build, Some(SwarmId::PLAYER),));
}

#[test]
fn demand_pressure_creates_planned_production_facility() {
    // Acceptance: "Production pressure creates a Planned
    // Production Facility instead of an instant completed
    // facility." A swarm with high unmet demand and an
    // owned Build Zone must see a `PlannedStructure` of
    // `PlannedKind::ProductionFacility` emerge from the
    // auto-creation system, NOT a completed
    // `ProductionFacility`. The plan is the visible
    // "demand noticed, build scheduled" state.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }
    paint_build(&mut app, IVec2::new(0, 0));

    app.update();

    let world = app.world_mut();
    // The plan exists.
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::ProductionFacility)
        .count();
    assert_eq!(
        planned_count, 1,
        "demand pressure must create exactly one Planned Production Facility"
    );
    // The plan carries the sidecar recording the first
    // target the completed facility should produce. With
    // all three types at equal deficit, the picker's
    // stable tie-break order picks Worker.
    let plan_entity = world
        .query::<(Entity, &PlannedStructure)>()
        .iter(world)
        .find(|(_, p)| p.kind == PlannedKind::ProductionFacility)
        .map(|(e, _)| e)
        .unwrap();
    let plan_target = world
        .entity(plan_entity)
        .get::<PlannedProductionTarget>()
        .expect("Planned Production Facility must carry a PlannedProductionTarget sidecar")
        .0;
    assert_eq!(
        plan_target,
        NanobotType::Worker,
        "deficit tie-break must land on Worker (first in NanobotType::ALL)"
    );
    // No completed Production Facility exists yet: the
    // build is worker-time only.
    let facility_count = world.query::<&ProductionFacility>().iter(world).count();
    assert_eq!(
        facility_count, 0,
        "no completed Production Facility must exist before a Worker builds the plan"
    );
}

#[test]
fn planned_production_facility_uses_planned_visual() {
    // Acceptance: "Planned Structures are visibly distinct
    // from completed structures." A newly-planned
    // Production Facility must carry the planned visual
    // color so the player can tell the structure is not
    // yet built.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }
    paint_build(&mut app, IVec2::new(0, 0));

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Sprite)>();
    let (planned, sprite) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::ProductionFacility)
        .expect("Planned Production Facility must exist");
    assert_eq!(planned.cell, IVec2::new(0, 0));
    assert_eq!(
        sprite.color,
        planned_visual_color(),
        "Planned Production Facility must use the planned visual color"
    );
}

#[test]
fn planned_production_facility_is_owned_by_swarm_that_painted_build_cell() {
    // Acceptance: "Planned Production Facility placement
    // is constrained to an owned Build Zone." A
    // player-painted Build cell produces a Planned
    // Production Facility stamped with the player
    // `OwnerSwarm`. The completed facility keeps the
    // same ownership.
    let mut app = build_app();
    let center = common::cell_world_center(IVec2::new(0, 0));
    let swarm = common::spawn_swarm_at(&mut app, center);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }
    paint_build(&mut app, IVec2::new(0, 0));

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &OwnerSwarm)>();
    let (planned, owner) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::ProductionFacility)
        .expect("Planned Production Facility must exist when a swarm is present");
    assert_eq!(planned.kind, PlannedKind::ProductionFacility);
    assert_eq!(
        owner.0, swarm,
        "Planned Production Facility must be owned by the swarm that painted the Build Zone"
    );
}

#[test]
fn no_planned_production_facility_without_build_zone() {
    // Acceptance: "No new Production Facility is planned
    // when no suitable Build Zone exists." A swarm with
    // high unmet demand and no Build paint does NOT
    // spawn a Planned Production Facility. The Build
    // Zone is the placement constraint; without it the
    // demand layer is a no-op.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }
    // No Build paint.

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::ProductionFacility)
        .count();
    assert_eq!(
        planned_count, 0,
        "no Planned Production Facility must exist without an owned Build Zone"
    );
    let facility_count = world.query::<&ProductionFacility>().iter(world).count();
    assert_eq!(
        facility_count, 0,
        "no completed Production Facility must exist without an owned Build Zone"
    );
}

#[test]
fn worker_claims_planned_production_facility() {
    // Acceptance: "One Worker builds the planned facility
    // to completion." The "one Worker" half of the
    // contract: an idle Worker at the planned cell
    // receives a `PlannedStructureClaim` and the plan
    // records the worker as its `active_worker`. Other
    // workers cannot double-claim the same plan.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let plan =
        common::spawn_planned_production_facility_at_cell(&mut app, cell, NanobotType::Worker);
    let _worker = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let claim = world
        .entity(_worker)
        .get::<PlannedStructureClaim>()
        .expect("idle worker must claim the planned production facility");
    assert_eq!(claim.target, plan);
    let planned = world.entity(plan).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned.active_worker,
        Some(_worker),
        "planned Production Facility must record the worker as active_worker"
    );
}

#[test]
fn only_one_worker_claims_a_planned_production_facility() {
    // "Other Workers do not work on an already claimed
    // Planned Structure." Two idle workers and one planned
    // Production Facility: only one worker ends up with a
    // claim; the other stays idle. Mirrors the planned
    // structure foundation's "at most one worker" contract
    // for the new kind.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let plan =
        common::spawn_planned_production_facility_at_cell(&mut app, cell, NanobotType::Worker);
    let worker_a = common::spawn_worker_at(&mut app, center);
    let worker_b = common::spawn_worker_at(&mut app, center);

    app.update();

    let world = app.world();
    let planned = world.entity(plan).get::<PlannedStructure>().unwrap();
    let active = planned
        .active_worker
        .expect("planned Production Facility must be claimed");
    assert!(
        active == worker_a || active == worker_b,
        "active worker must be one of the two idle workers"
    );
    let claim_a = world
        .entity(worker_a)
        .get::<PlannedStructureClaim>()
        .is_some();
    let claim_b = world
        .entity(worker_b)
        .get::<PlannedStructureClaim>()
        .is_some();
    let claim_count = (claim_a as u32) + (claim_b as u32);
    assert_eq!(
        claim_count, 1,
        "exactly one worker must hold the claim; got a={} b={}",
        claim_a, claim_b
    );
}

#[test]
fn worker_builds_planned_production_facility_to_completion() {
    // Acceptance: "One Worker builds the planned facility
    // to completion." A Worker at the plan's cell claims
    // it, spends `DEFAULT_PLANNED_WORK_TICKS` ticks, and
    // the plan promotes to a completed `ProductionFacility`
    // (with its `current_target` round-tripped from the
    // sidecar). The visual flips to the completed color.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let plan =
        common::spawn_planned_production_facility_at_cell(&mut app, cell, NanobotType::Hauler);
    let _worker = common::spawn_worker_at(&mut app, center);

    // 1 tick for claim + arrive (worker is already at the
    // cell, so the arrive system fires on the same tick
    // as the claim), then `DEFAULT_PLANNED_WORK_TICKS`
    // ticks of work. The build completes on the
    // `DEFAULT_PLANNED_WORK_TICKS + 1`-th tick. We do
    // NOT add a buffer here: the completed facility's
    // production cycle starts immediately, and the
    // work system resets `current_target` to `None` when
    // the cycle completes. The sidecar round-trip must
    // be checked before the production cycle finishes.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize;
    for _ in 0..build_ticks {
        app.update();
    }

    let world = app.world_mut();
    // PlannedStructure is removed on promotion.
    assert!(
        world.entity(plan).get::<PlannedStructure>().is_none(),
        "PlannedStructure must be removed on completion"
    );
    // Completion creates an empty terminal; production cannot begin until
    // physical logistics pays the first cycle cost.
    let facility = world
        .entity(plan)
        .get::<ProductionFacility>()
        .expect("completion must replace PlannedStructure with a ProductionFacility");
    assert_eq!(
        facility.current_target, None,
        "completed facility must not receive a free first production cycle",
    );
    assert_eq!(
        facility.progress, 0,
        "completed facility must start a fresh production cycle at progress 0"
    );
    assert!(
        world
            .entity(plan)
            .get::<PlannedProductionTarget>()
            .is_none(),
        "PlannedProductionTarget sidecar must be removed on completion"
    );
    // The visual flipped to the completed color.
    let sprite = world
        .entity(plan)
        .get::<Sprite>()
        .expect("completed Production Facility must carry a Sprite");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed visual must use the completed color"
    );
    // The completed facility is a terminal consumer: it owns its
    // input hopper on the ProductionFacility component and carries
    // NO Stockpile, so it never enters stockpile queries (a gather
    // worker cannot dump into it, a hauler cannot pick it as a
    // stockpile source/sink). The hopper starts empty -- leg 3
    // haulers deliver into it.
    assert!(
        world.entity(plan).get::<Stockpile>().is_none(),
        "completed Production Facility must NOT carry a Stockpile; it owns an input hopper instead"
    );
    let completed_facility = world
        .entity(plan)
        .get::<ProductionFacility>()
        .expect("completed Production Facility must carry a ProductionFacility");
    assert_eq!(
        completed_facility.input_amount, 0,
        "input hopper starts empty; leg 3 haulers deliver into it"
    );
}

#[test]
fn completed_production_facility_consumes_resources_and_produces_nanobots() {
    // Acceptance: "Completed Production Facilities consume
    // resources and produce nanobots through existing
    // production rules." A planned facility is built by a
    // Worker, then logistics fills its terminal input, a full
    // production cycle runs, and a new nanobot spawns.
    let mut app = build_app();
    let swarm_center = Vec2::new(0.0, 0.0);
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, swarm_center);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 1);
        ratio.set_weight(NanobotType::Hauler, 1);
    }
    let plan =
        common::spawn_planned_production_facility_at_cell(&mut app, cell, NanobotType::Hauler);
    let _worker = common::spawn_worker_at(&mut app, cell_center);

    // Drive the build to completion, then run enough
    // ticks for the completed facility to do one full
    // production cycle.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 2;
    for _ in 0..build_ticks {
        app.update();
    }
    // The planned structure has promoted to a real
    // facility by now.
    let world = app.world_mut();
    assert!(
        world
            .query::<&ProductionFacility>()
            .iter(world)
            .next()
            .is_some(),
        "completed Production Facility must exist after the build"
    );
    common::fill_facility_input(&mut app, plan);
    // Drive one full production cycle on the completed
    // facility. PRODUCTION_COST_PER_BOT = 20 is
    // consumed, then PRODUCTION_TICKS_PER_BOT = 5 work
    // ticks, then a nanobot spawns.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::PRODUCTION_TICKS_PER_BOT;
    let production_ticks = 1 + PRODUCTION_TICKS_PER_BOT as usize + 2;
    for _ in 0..production_ticks {
        app.update();
    }

    // A new Worker owned by the swarm must exist.
    // Issue #38 / ADR-0004: production-spawned
    // nanobots are top-level entities, not children of the swarm. The
    // existing Worker leaves a Hauler composition deficit.
    let swarm_id = app
        .world()
        .entity(swarm)
        .get::<SwarmId>()
        .copied()
        .expect("swarm must carry a SwarmId");
    let mut owned_haulers_after = 0;
    {
        let world = app.world_mut();
        let mut query = world.query::<(
            &NanobotType,
            &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
        )>();
        for (ty, member) in query.iter(world) {
            if member.0 == swarm_id && *ty == NanobotType::Hauler {
                owned_haulers_after += 1;
            }
        }
    }
    assert_eq!(
        owned_haulers_after, 1,
        "completed Production Facility must produce only after its first cycle is paid",
    );
}

#[test]
fn existing_starting_scenario_facilities_remain_valid_seed_structures() {
    // Acceptance: "Existing starting scenario facilities
    // remain valid seed structures." The default
    // scenario's seed `ProductionFacility` is a real
    // `ProductionFacility` (not a planned one) and must
    // remain functional. Painting a Build Zone next to
    // the seed facility must NOT turn the seed into a
    // planned structure, and the seed's first pick cycle
    // must run through the existing production systems.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::PRODUCTION_COST_PER_BOT;
    let mut app = common::sim_app_with_production_planned();
    app.insert_resource(ProductionRatio::new());
    let swarm_center = Vec2::new(0.0, 0.0);
    let _swarm = common::spawn_swarm_at(&mut app, swarm_center);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 1);
    }
    // Seed facility owned by the swarm, with the same
    // shape as `scenario::spawn_production_facility`.
    let seed = common::spawn_facility_at(&mut app, _swarm, swarm_center);
    // Stockpile with material so the seed's first pick
    // cycle can pull from it.
    let _stockpile =
        common::spawn_stockpile(&mut app, swarm_center, PRODUCTION_COST_PER_BOT * 5, 1000);
    // No Build paint; the seed must not be turned into a
    // plan.
    app.update();
    {
        let world = app.world_mut();
        let seed_facility = world
            .entity(seed)
            .get::<ProductionFacility>()
            .expect("seed facility must still be a ProductionFacility (not a plan)");
        assert!(
            seed_facility.is_busy() || seed_facility.current_target.is_some(),
            "seed facility must start producing on the first tick (existing production rules)"
        );
        let planned_count = world
            .query::<&PlannedStructure>()
            .iter(world)
            .filter(|p| p.kind == PlannedKind::ProductionFacility)
            .count();
        assert_eq!(
            planned_count, 0,
            "seed Production Facility must not become a PlannedStructure"
        );
    }
}

#[test]
fn plan_does_not_pile_under_repeated_demand_ticks() {
    // Demand-driven robustness: even when the demand
    // stays high across many ticks, the auto-creation
    // system does not pile a second plan in the same
    // cell, and does not plan a second facility
    // elsewhere while the first plan is still pending.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
    }
    paint_build(&mut app, IVec2::new(0, 0));

    for _ in 0..20 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::ProductionFacility)
        .count();
    assert_eq!(
        planned_count, 1,
        "auto-creation must not pile multiple Planned Production Facilities in the same cell"
    );
    // The completed facility count is 0 (the plan has
    // not been built by any worker in this test).
    let facility_count = world.query::<&ProductionFacility>().iter(world).count();
    assert_eq!(
        facility_count, 0,
        "no completed Production Facility must appear without a Worker building the plan"
    );
}

#[test]
fn planned_kind_includes_production_facility() {
    // Pin the `PlannedKind::ALL` / `PlannedKind::COUNT`
    // contract: the new variant shows up in the
    // foundation's stable iteration list, with a stable
    // index distinct from the Source and Sink
    // Stockpile variants.
    let kinds: Vec<PlannedKind> = PlannedKind::ALL.to_vec();
    assert_eq!(kinds.len(), PlannedKind::COUNT);
    assert!(kinds.contains(&PlannedKind::SourceStockpile));
    assert!(kinds.contains(&PlannedKind::SinkStockpile));
    assert!(
        kinds.contains(&PlannedKind::ProductionFacility),
        "PlannedKind::ALL must include ProductionFacility"
    );
    let production_index = PlannedKind::ProductionFacility.index();
    let source_index = PlannedKind::SourceStockpile.index();
    let sink_index = PlannedKind::SinkStockpile.index();
    assert_ne!(production_index, source_index);
    assert_ne!(production_index, sink_index);
    assert_ne!(source_index, sink_index);
}

#[test]
fn idle_worker_at_planned_production_facility_claims_and_works() {
    // Pin the end-to-end "claim -> arrive -> progress"
    // chain for a Planned Production Facility. An
    // idle Worker placed at the cell receives a
    // `PlannedStructureClaim` and reaches the
    // `PlannedStructureProgress` state after a few
    // ticks.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let center = common::cell_world_center(cell);
    let _plan =
        common::spawn_planned_production_facility_at_cell(&mut app, cell, NanobotType::Worker);
    let worker = common::spawn_worker_at(&mut app, center);

    for _ in 0..3 {
        app.update();
    }

    let world = app.world();
    assert!(
        world
            .entity(worker)
            .get::<PlannedStructureClaim>()
            .is_some(),
        "idle worker must claim the Planned Production Facility"
    );
    assert!(
        world
            .entity(worker)
            .get::<PlannedStructureProgress>()
            .is_some(),
        "worker at the cell must be in progress after a few ticks"
    );
}
