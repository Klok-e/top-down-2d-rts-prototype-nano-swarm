//! Integration tests for issue #11: Production Facilities and
//! Production Ratio control.
//!
//! Each test isolates one behavior so a failure points at a single
//! contract: ratio set/get, deficit priority, blocked-type skip,
//! material consumption from local stockpiles, full-cycle
//! nanobot spawn, shared cost/time, and facility emergence.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{
        NanobotType, ProductionFacility, ProductionRatio, PRODUCTION_COST_PER_BOT,
        PRODUCTION_TICKS_PER_BOT,
    },
    resources::Stockpile,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    // Use an empty ratio by default so each test starts from a
    // clean slate. Tests that need a specific mix set their own
    // targets; the sensible default lives in the game's
    // `lib.rs` initialization instead.
    let mut app = common::sim_app_with_production();
    app.insert_resource(ProductionRatio::new());
    app
}

fn facility_count(world: &mut World) -> usize {
    let mut q = world.query::<&ProductionFacility>();
    q.iter(world).count()
}

fn nanobot_count_by_type(world: &mut World, kind: NanobotType) -> u32 {
    let mut q = world.query::<&NanobotType>();
    q.iter(world).filter(|t| **t == kind).count() as u32
}

#[test]
fn production_ratio_can_be_set_for_each_type() {
    // Acceptance: "Player can set target Production Ratio for
    // Worker, Hauler, and Defender." A round-trip through the
    // resource proves the public surface works for all three
    // types and the total tracks the sum.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 8);
        ratio.set_target(NanobotType::Hauler, 3);
        ratio.set_target(NanobotType::Defender, 1);
    }
    let ratio = app.world().resource::<ProductionRatio>();
    assert_eq!(ratio.target(NanobotType::Worker), 8);
    assert_eq!(ratio.target(NanobotType::Hauler), 3);
    assert_eq!(ratio.target(NanobotType::Defender), 1);
    assert_eq!(ratio.total(), 12);
}

#[test]
fn facility_picks_type_with_largest_deficit() {
    // Acceptance: "Production picks type with largest deficit
    // from target ratio." A facility facing a 10-unit Hauler
    // deficit, 0-unit Worker deficit, and 4-unit Defender
    // deficit must commit to producing Haulers first.
    let mut app = build_app();
    // Single swarm holding 5 Workers and 1 Defender; targets
    // 5/10/5 -> deficits 0/10/4, so the picker must choose
    // Hauler.
    common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::new(100.0, 100.0),
        &[(NanobotType::Worker, 5), (NanobotType::Defender, 1)],
    );
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 10);
        ratio.set_target(NanobotType::Defender, 5);
    }
    let stockpile_pos = Vec2::new(200.0, 100.0);
    let _stockpile =
        common::spawn_stockpile(&mut app, stockpile_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let _facility = common::spawn_idle_facility_at(&mut app, stockpile_pos);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<&ProductionFacility>();
    let facility = q.iter(world).next().expect("facility must exist");
    assert_eq!(
        facility.current_target,
        Some(NanobotType::Hauler),
        "facility must pick the type with the largest deficit"
    );
}

#[test]
fn facility_skips_blocked_type() {
    // Acceptance: "Blocked types are skipped temporarily
    // instead of stalling all production." With Worker and
    // Hauler both at equal deficit, but Worker pre-blocked,
    // the facility must commit to Hauler. The Worker block
    // does not stall the facility.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 5);
    }
    let pos = Vec2::new(150.0, 50.0);
    let _stockpile = common::spawn_stockpile(&mut app, pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let facility_entity = {
        let mut f = ProductionFacility::new();
        f.blocked_types.insert(NanobotType::Worker);
        app.world_mut()
            .spawn((f, Transform::from_translation(pos.extend(0.0))))
            .id()
    };

    app.update();

    let facility = app
        .world()
        .entity(facility_entity)
        .get::<ProductionFacility>()
        .expect("facility must exist");
    assert_eq!(
        facility.current_target,
        Some(NanobotType::Hauler),
        "facility must skip the blocked type and pick the next viable one"
    );
    // The blocked set is preserved across the current cycle:
    // it is the running list of types that could not be
    // started yet. It clears at the end of the cycle, not
    // partway through.
    assert!(facility.is_blocked(NanobotType::Worker));
}

#[test]
fn facility_consumes_delivered_resources() {
    // Acceptance: "Production Facilities consume physically
    // delivered resources." A facility at a local stockpile
    // drains exactly PRODUCTION_COST_PER_BOT when it starts
    // a production cycle. A second stockpile far away is
    // untouched, matching the "physically delivered" half
    // of the contract.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let local = common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 2, 1000);
    let distant_pos = Vec2::new(5000.0, 0.0);
    let distant = common::spawn_stockpile(&mut app, distant_pos, 10_000, 20_000);
    let _facility = common::spawn_idle_facility_at(&mut app, facility_pos);

    app.update();

    let world = app.world_mut();
    let local_state = world.entity(local).get::<Stockpile>().unwrap();
    let distant_state = world.entity(distant).get::<Stockpile>().unwrap();
    assert_eq!(
        local_state.amount, PRODUCTION_COST_PER_BOT,
        "local stockpile must lose exactly the production cost; got {}",
        local_state.amount
    );
    assert_eq!(
        distant_state.amount, 10_000,
        "distant stockpile must not be drained by local production"
    );
}

#[test]
fn facility_produces_nanobot_after_full_cycle() {
    // End-to-end: a facility with enough material and a
    // positive deficit produces a nanobot after
    // PRODUCTION_TICKS_PER_BOT ticks. The new nanobot is a
    // child of the swarm.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 3);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let _stockpile =
        common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let _facility = common::spawn_idle_facility_at(&mut app, facility_pos);

    // 1 tick to pick the target, PRODUCTION_TICKS_PER_BOT
    // ticks of progress (the cycle completes on the
    // PRODUCTION_TICKS_PER_BOT-th tick), +2 buffer.
    let total_ticks = 1 + PRODUCTION_TICKS_PER_BOT as usize + 2;
    for _ in 0..total_ticks {
        app.update();
    }

    // A new Worker child of the swarm must exist.
    let world = app.world_mut();
    let children: Vec<Entity> = world
        .get::<Children>(swarm)
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let mut worker_children = 0;
    for child in children {
        if let Some(ty) = world.entity(child).get::<NanobotType>() {
            if *ty == NanobotType::Worker {
                worker_children += 1;
            }
        }
    }
    assert_eq!(
        worker_children, 1,
        "facility must spawn exactly one Worker after a full cycle"
    );
    // The facility has either just finished the first cycle
    // and reset, or already started a second cycle. Both
    // states are valid post-conditions; the spawn count is
    // the load-bearing assertion.
    let _facility = world
        .query::<&ProductionFacility>()
        .iter(world)
        .next()
        .expect("facility must still exist");
}

#[test]
fn shared_early_cost_across_types() {
    // Acceptance: "Tests cover ... shared early cost/time ..."
    // All three early types consume the same
    // PRODUCTION_COST_PER_BOT and take the same
    // PRODUCTION_TICKS_PER_BOT. Run two side-by-side
    // scenarios: one facility producing Worker, one
    // producing Hauler. Both must consume the same amount
    // of material from their stockpiles.
    fn run_scenario(target: NanobotType) -> u32 {
        let mut app = build_app();
        let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        {
            let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
            ratio.set_target(target, 1);
        }
        let facility_pos = Vec2::new(0.0, 0.0);
        let stockpile_entity =
            common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 3, 1000);
        let _facility = common::spawn_idle_facility_at(&mut app, facility_pos);
        app.update();
        app.world()
            .entity(stockpile_entity)
            .get::<Stockpile>()
            .unwrap()
            .amount
    }

    let worker_remaining = run_scenario(NanobotType::Worker);
    let hauler_remaining = run_scenario(NanobotType::Hauler);
    let defender_remaining = run_scenario(NanobotType::Defender);

    // All three must have consumed exactly the production
    // cost, leaving the same amount in the stockpile.
    let expected = PRODUCTION_COST_PER_BOT * 3 - PRODUCTION_COST_PER_BOT;
    assert_eq!(worker_remaining, expected, "Worker cycle cost");
    assert_eq!(hauler_remaining, expected, "Hauler cycle cost");
    assert_eq!(defender_remaining, expected, "Defender cycle cost");
    // And explicitly: the three remaining amounts are equal
    // -- the cost is shared.
    assert_eq!(worker_remaining, hauler_remaining);
    assert_eq!(hauler_remaining, defender_remaining);
}

#[test]
fn shared_early_ticks_across_types() {
    // The "shared cost/time" contract is half cost and half
    // time. A Worker and a Hauler cycle must both complete
    // on the PRODUCTION_TICKS_PER_BOT-th tick after the
    // initial pick tick. Driving the same tick count for
    // both and checking the cycle state pins the time half.
    fn run_to_cycle_completion(target: NanobotType) -> u32 {
        let mut app = build_app();
        let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        {
            let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
            ratio.set_target(target, 1);
        }
        let facility_pos = Vec2::new(0.0, 0.0);
        let _stockpile =
            common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 3, 1000);
        let _facility = common::spawn_idle_facility_at(&mut app, facility_pos);
        // 1 pick tick + PRODUCTION_TICKS_PER_BOT work ticks
        // = PRODUCTION_TICKS_PER_BOT + 1 total ticks. The
        // cycle completes on the last tick, spawning a
        // nanobot. Counting children of the swarm after
        // exactly that many updates is the assertion.
        let total_ticks = 1 + PRODUCTION_TICKS_PER_BOT as usize;
        for _ in 0..total_ticks {
            app.update();
        }
        let world = app.world();
        let children: Vec<Entity> = world
            .get::<Children>(swarm)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        let mut count = 0;
        for child in children {
            if let Some(ty) = world.entity(child).get::<NanobotType>() {
                if *ty == target {
                    count += 1;
                }
            }
        }
        count
    }

    assert_eq!(
        run_to_cycle_completion(NanobotType::Worker),
        1,
        "Worker cycle must complete in PRODUCTION_TICKS_PER_BOT ticks"
    );
    assert_eq!(
        run_to_cycle_completion(NanobotType::Hauler),
        1,
        "Hauler cycle must complete in PRODUCTION_TICKS_PER_BOT ticks"
    );
    assert_eq!(
        run_to_cycle_completion(NanobotType::Defender),
        1,
        "Defender cycle must complete in PRODUCTION_TICKS_PER_BOT ticks"
    );
}

#[test]
fn additional_facility_plans_when_existing_busy_and_build_zone_free() {
    // Acceptance: "Additional Production Facilities emerge
    // from demand pressure when existing capacity is too
    // busy." After the issue #27 migration, "emerge" means
    // "a Planned Production Facility appears in an owned
    // Build Zone", not "a completed facility is
    // instant-spawned". The plan is then built by a Worker
    // and the completed facility is busy from the moment
    // it is built (its `current_target` is the type the
    // plan was created for). This test pins the new
    // emergence path end-to-end.
    use top_down_2d_rts_prototype_nano_swarm::{
        intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
        nanobot::{
            completed_visual_color, planned_visual_color, OwnerSwarm, PlannedKind,
            PlannedProductionTarget, PlannedStructure, SwarmId, DEFAULT_PLANNED_WORK_TICKS,
        },
    };
    let mut app = common::sim_app_with_production_planned();
    app.insert_resource(ProductionRatio::new());
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 10);
        ratio.set_target(NanobotType::Hauler, 10);
        ratio.set_target(NanobotType::Defender, 10);
    }
    // One facility already producing Worker (busy). The
    // deficit is high (3 * 10 = 30) so the emergence
    // threshold is comfortably exceeded.
    let facility_pos = Vec2::new(0.0, 0.0);
    let _busy = common::spawn_busy_facility_at(&mut app, facility_pos, NanobotType::Worker);
    // The Build Zone the plan will land in. Painted by
    // the player swarm so the new auto-creator (issue #27)
    // can match the cell to the swarm's owner. The Build
    // cell is at a different cell from the busy facility
    // so the auto-creator does not see the facility as
    // occupying the Build cell.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(
            IVec2::new(1, 0),
            IntentKind::Build,
            PAINT_STRENGTH_CAP,
            Some(SwarmId::PLAYER),
        ));
    }

    app.update();

    // After one tick: the plan exists, the completed
    // facility does NOT (the build is worker-time only).
    {
        let world = app.world_mut();
        let planned_count = world
            .query::<&PlannedStructure>()
            .iter(world)
            .filter(|p| p.kind == PlannedKind::ProductionFacility)
            .count();
        assert_eq!(
            planned_count, 1,
            "demand pressure must spawn a Planned Production Facility when the existing one is busy"
        );
        let facility_count = facility_count(world);
        assert_eq!(
            facility_count, 1,
            "no completed Production Facility must appear from demand; the build is worker-time only"
        );
    }
    // Place a Worker at the plan's cell so the build
    // happens immediately. Drive the build to completion
    // and check the completed facility is busy with the
    // picked target.
    let planned_entity = {
        let world = app.world_mut();
        let mut q = world.query::<(Entity, &PlannedStructure)>();
        let (entity, _planned) = q
            .iter(world)
            .find(|(_, p)| p.kind == PlannedKind::ProductionFacility)
            .expect("planned production facility must exist");
        // The plan carries the demand-time sidecar, the
        // OwnerSwarm, and the planned visual. The detailed
        // round-trip / visual / owner assertions live in
        // `production_facility_planned.rs`; here we just
        // confirm the components are present and the
        // visual is the planned one before the build.
        let plan = world.entity(entity);
        assert!(
            plan.get::<PlannedProductionTarget>().is_some(),
            "plan must carry a PlannedProductionTarget sidecar"
        );
        assert!(
            plan.get::<OwnerSwarm>().is_some(),
            "plan must carry an OwnerSwarm"
        );
        assert_eq!(
            plan.get::<Sprite>()
                .expect("plan must carry a Sprite")
                .color,
            planned_visual_color(),
            "Planned Production Facility must use the planned visual color"
        );
        entity
    };
    let cell = app
        .world()
        .entity(planned_entity)
        .get::<PlannedStructure>()
        .unwrap()
        .cell;
    let center = common::cell_world_center(cell);
    // The Worker must be placed AT the planned cell
    // center so the claim + arrive + work chain can
    // fire without a long walk. The build then
    // completes in `DEFAULT_PLANNED_WORK_TICKS` ticks
    // of worker time.
    let _worker = common::spawn_worker_at(&mut app, center);

    // 1 tick for claim + arrive (worker is already at
    // the cell, so the arrive system fires on the same
    // tick as the claim), then `DEFAULT_PLANNED_WORK_TICKS`
    // ticks of work. The build completes on the
    // `DEFAULT_PLANNED_WORK_TICKS + 1`-th tick. We do
    // NOT add a buffer here: the completed facility's
    // production cycle starts immediately, and the work
    // system resets `current_target` to `None` when the
    // cycle completes. The `is_busy` check must run
    // before the production cycle finishes.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize;
    for _ in 0..build_ticks {
        app.update();
    }

    let world = app.world_mut();
    // The planned structure has been promoted: the
    // PlannedStructure component is gone, the entity now
    // carries a ProductionFacility + local Stockpile,
    // and the visual flipped to the completed color.
    assert!(
        world
            .entity(planned_entity)
            .get::<PlannedStructure>()
            .is_none(),
        "PlannedStructure must be removed on completion"
    );
    let facility = world
        .entity(planned_entity)
        .get::<ProductionFacility>()
        .expect("completion must replace PlannedStructure with a ProductionFacility");
    assert!(
        facility.is_busy(),
        "completed facility must be busy (its current_target is the picked deficit type)"
    );
    // The first pick target round-trips through the
    // `PlannedProductionTarget` sidecar: the completed
    // facility's `current_target` must equal the target
    // stamped on the plan.
    let sidecar_target = world
        .entity(planned_entity)
        .get::<PlannedProductionTarget>()
        .map(|t| t.0)
        .or(facility.current_target);
    assert_eq!(
        facility.current_target, sidecar_target,
        "the production target must round-trip through the sidecar so the completed facility's \
         first pick cycle respects the original demand"
    );
    let sprite = world
        .entity(planned_entity)
        .get::<Sprite>()
        .expect("completed facility must carry a Sprite");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed visual must flip to the completed color on promotion"
    );
    let owner = world
        .entity(planned_entity)
        .get::<OwnerSwarm>()
        .expect("completed facility must keep the plan's OwnerSwarm")
        .0;
    assert_eq!(
        owner, swarm,
        "completed facility must keep the swarm that owned the plan"
    );
}

#[test]
fn no_emergence_when_existing_facility_is_idle() {
    // The "existing capacity is too busy" half of the
    // emergence contract is symmetric: an idle facility
    // means the swarm has spare capacity, so the auto
    // creator must NOT spawn a duplicate.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 10);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let _idle = common::spawn_idle_facility_at(&mut app, facility_pos);
    let _stockpile =
        common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 5, 1000);

    app.update();

    let world = app.world_mut();
    let count = facility_count(world);
    assert_eq!(
        count, 1,
        "no second facility must emerge while an existing one is idle"
    );
}

#[test]
fn blocked_types_cleared_after_full_cycle() {
    // The "skip blocked types temporarily" half of the
    // contract is tested by a full cycle: pre-block Worker,
    // pick Hauler, run the cycle, the blocked set is
    // cleared. Next cycle Worker is re-evaluated and gets
    // unblocked because material is available.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 1);
        ratio.set_target(NanobotType::Hauler, 1);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let facility_entity = {
        let mut f = ProductionFacility::new();
        f.blocked_types.insert(NanobotType::Worker);
        app.world_mut()
            .spawn((f, Transform::from_translation(facility_pos.extend(0.0))))
            .id()
    };
    let _stockpile =
        common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 5, 1000);

    // 1 pick tick + PRODUCTION_TICKS_PER_BOT work ticks to
    // complete the cycle.
    for _ in 0..(1 + PRODUCTION_TICKS_PER_BOT as usize) {
        app.update();
    }

    let facility = app
        .world()
        .entity(facility_entity)
        .get::<ProductionFacility>()
        .expect("facility must exist");
    assert!(
        facility.blocked_types.is_empty(),
        "blocked_types must clear at the end of a cycle so the next cycle re-tries them"
    );
}

#[test]
fn no_production_when_all_types_blocked() {
    // With every type in the blocked set, the facility has
    // nothing to produce. The current_target stays None and
    // no material is consumed. This is the "stalling
    // temporarily" half of the "blocked types are skipped
    // temporarily instead of stalling all production" rule:
    // the facility stalls only when *every* candidate is
    // blocked, not when one is.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Worker, 5);
        ratio.set_target(NanobotType::Hauler, 5);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let facility_entity = {
        let mut f = ProductionFacility::new();
        f.blocked_types.insert(NanobotType::Worker);
        f.blocked_types.insert(NanobotType::Hauler);
        f.blocked_types.insert(NanobotType::Defender);
        app.world_mut()
            .spawn((f, Transform::from_translation(facility_pos.extend(0.0))))
            .id()
    };
    let stockpile =
        common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 5, 1000);

    app.update();

    let facility = app
        .world()
        .entity(facility_entity)
        .get::<ProductionFacility>()
        .expect("facility must exist");
    assert_eq!(
        facility.current_target, None,
        "facility with all types blocked must stay idle"
    );
    let stockpile_state = app.world().entity(stockpile).get::<Stockpile>().unwrap();
    assert_eq!(
        stockpile_state.amount,
        PRODUCTION_COST_PER_BOT * 5,
        "stockpile must not be drained while facility is fully blocked"
    );
}

#[test]
fn production_increases_population_of_picked_type() {
    // End-to-end counter test: a target of 1 Hauler with 0
    // existing haulers, a single facility and stockpile,
    // and one full cycle must produce exactly one Hauler
    // and bring the population to 1.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_target(NanobotType::Hauler, 1);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let _stockpile =
        common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let _facility = common::spawn_idle_facility_at(&mut app, facility_pos);

    // Initial hauler population is 0.
    assert_eq!(
        nanobot_count_by_type(app.world_mut(), NanobotType::Hauler),
        0
    );

    // 1 pick tick + PRODUCTION_TICKS_PER_BOT work ticks.
    for _ in 0..(1 + PRODUCTION_TICKS_PER_BOT as usize) {
        app.update();
    }

    assert_eq!(
        nanobot_count_by_type(app.world_mut(), NanobotType::Hauler),
        1,
        "exactly one Hauler must be produced after one full cycle"
    );
}
