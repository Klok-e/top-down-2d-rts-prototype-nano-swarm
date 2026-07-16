//! Integration tests for issue #11: Production Facilities and
//! Production Ratio control.
//!
//! Each test isolates one behavior so a failure points at a single
//! contract: ratio set/get, deficit priority, blocked-type skip,
//! material consumption from local stockpiles, full-cycle
//! nanobot spawn, shared cost/time, and facility emergence.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, NanobotType, OwnerSwarm, PRODUCTION_COST_PER_BOT, PRODUCTION_TICKS_PER_BOT,
        PopulationDemandPlugin, ProductionFacility, ProductionRatio,
        SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind, SwarmBundle, SwarmId,
        SwarmMember, production_facility_pick_target_system,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile},
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
        ratio.set_weight(NanobotType::Worker, 8);
        ratio.set_weight(NanobotType::Hauler, 3);
        ratio.set_weight(NanobotType::Defender, 1);
    }
    let ratio = app.world().resource::<ProductionRatio>();
    assert_eq!(ratio.weight(NanobotType::Worker), 8);
    assert_eq!(ratio.weight(NanobotType::Hauler), 3);
    assert_eq!(ratio.weight(NanobotType::Defender), 1);
    assert_eq!(ratio.total_weight(), 12);
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
        ratio.set_weight(NanobotType::Worker, 5);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 5);
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
        ratio.set_weight(NanobotType::Worker, 5);
        ratio.set_weight(NanobotType::Hauler, 5);
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
    common::fill_facility_input(&mut app, facility_entity);

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
    // delivered resources." Under the tiered logistics flow
    // (ADR-0005) production consumes exclusively from the
    // facility's own input hopper -- the buffer haulers fill
    // in leg 3. Starting from a full hopper, one pick cycle
    // drains exactly PRODUCTION_COST_PER_BOT. A stockpile
    // sitting right next to the facility is NOT touched,
    // because production no longer scans stockpiles; this is
    // what makes leg 3 non-bypassable.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 5);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let facility = common::spawn_idle_facility_at(&mut app, facility_pos);
    // A source stockpile right next to the facility must NOT
    // be drained: production reads only the facility input.
    let nearby = common::spawn_stockpile(&mut app, facility_pos, PRODUCTION_COST_PER_BOT * 2, 1000);
    let ledger_before = app
        .world()
        .resource::<ResourceLedger>()
        .total_for(SwarmId::PLAYER, ResourceKind::Minerals);
    let before = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap()
        .input_amount;

    app.update();

    let world = app.world_mut();
    let after = world
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap()
        .input_amount;
    assert_eq!(
        before - after,
        PRODUCTION_COST_PER_BOT,
        "facility input hopper must lose exactly the production cost; before={before} after={after}"
    );
    let nearby_state = world.entity(nearby).get::<Stockpile>().unwrap();
    assert_eq!(
        nearby_state.amount,
        PRODUCTION_COST_PER_BOT * 2,
        "nearby stockpile must not be drained; production reads only the facility input"
    );
    assert_eq!(
        world
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        ledger_before - PRODUCTION_COST_PER_BOT,
        "production removes consumed hopper material from owning swarm",
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
        ratio.set_weight(NanobotType::Worker, 3);
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

    // A new Worker owned by the swarm must exist.
    // Issue #38 / ADR-0004: production-spawned
    // nanobots are top-level entities, not children
    // of the swarm. Count Workers whose `SwarmMember`
    // matches the swarm's `SwarmId`.
    let swarm_id = app
        .world()
        .entity(swarm)
        .get::<SwarmId>()
        .copied()
        .expect("swarm must carry a SwarmId");
    let mut owned_workers = 0;
    {
        let world = app.world_mut();
        let mut query = world.query::<(
            &top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType,
            &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
        )>();
        for (ty, member) in query.iter(world) {
            if member.0 == swarm_id && *ty == NanobotType::Worker {
                owned_workers += 1;
            }
        }
    }
    assert_eq!(
        owned_workers, 1,
        "facility must spawn exactly one Worker after a full cycle"
    );
    // The facility has either just finished the first cycle
    // and reset, or already started a second cycle. Both
    // states are valid post-conditions; the spawn count is
    // the load-bearing assertion.
    let world = app.world_mut();
    let mut facilities_query = world.query::<&ProductionFacility>();
    let _facility = facilities_query
        .iter(world)
        .next()
        .expect("facility must still exist");
}

#[test]
fn produced_defender_enters_charge_lifecycle() {
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .resource_mut::<ProductionRatio>()
        .set_weight(NanobotType::Defender, 1);
    common::spawn_stockpile(&mut app, Vec2::ZERO, PRODUCTION_COST_PER_BOT * 2, 1000);
    common::spawn_idle_facility_at(&mut app, Vec2::ZERO);

    for _ in 0..(PRODUCTION_TICKS_PER_BOT + 3) {
        app.update();
    }

    let swarm_id = *app
        .world()
        .entity(swarm)
        .get::<SwarmId>()
        .expect("swarm has identity");
    let world = app.world_mut();
    let mut defenders = world.query::<(&NanobotType, &SwarmMember, Option<&Charge>)>();
    let charge = defenders
        .iter(world)
        .find_map(|(kind, member, charge)| {
            (*kind == NanobotType::Defender && member.0 == swarm_id).then_some(charge)
        })
        .expect("facility produced owned Defender");
    let charge = charge.expect("produced Defender must receive Charge");
    let full = Charge::default();
    assert_eq!(charge.current, full.current);
    assert_eq!(charge.max, full.max);
}

#[test]
fn degraded_facility_stops_operating_until_repaired() {
    let mut app = build_app();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .resource_mut::<ProductionRatio>()
        .set_weight(NanobotType::Worker, 1);
    let facility = common::spawn_idle_facility_at(&mut app, Vec2::ZERO);
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    app.world_mut().entity_mut(facility).insert(condition);
    let input_before = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap()
        .input_amount;

    app.update();

    let facility = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap();
    assert_eq!(facility.current_target, None);
    assert_eq!(facility.input_amount, input_before);
}

#[test]
fn exact_ratio_swarm_grows_when_useful_work_exceeds_population() {
    let mut app = build_app();
    app.add_plugins(PopulationDemandPlugin);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::ZERO);
    app.world_mut()
        .resource_mut::<ProductionRatio>()
        .set_weight(NanobotType::Worker, 1);
    for cell in [IVec2::ZERO, IVec2::new(1, 0)] {
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            cell,
            IntentKind::Gather,
            Some(SwarmId::PLAYER),
        );
        common::spawn_deposit(&mut app, common::cell_world_center(cell), 100);
    }
    let facility = common::spawn_idle_facility_at(&mut app, Vec2::ZERO);

    app.update();

    assert_eq!(
        app.world()
            .entity(facility)
            .get::<ProductionFacility>()
            .unwrap()
            .current_target,
        Some(NanobotType::Worker),
        "workload sets total growth while Production Ratio selects type",
    );
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
            ratio.set_weight(target, 1);
        }
        let facility_pos = Vec2::new(0.0, 0.0);
        let facility = common::spawn_idle_facility_at(&mut app, facility_pos);
        // Production consumes from the facility's own input
        // hopper, not from a stockpile. Capture the hopper
        // level before and after one pick cycle and return
        // the delta so the three types can be compared on
        // equal footing.
        let before = app
            .world()
            .entity(facility)
            .get::<ProductionFacility>()
            .unwrap()
            .input_amount;
        app.update();
        let after = app
            .world()
            .entity(facility)
            .get::<ProductionFacility>()
            .unwrap()
            .input_amount;
        before - after
    }

    let worker_consumed = run_scenario(NanobotType::Worker);
    let hauler_consumed = run_scenario(NanobotType::Hauler);
    let defender_consumed = run_scenario(NanobotType::Defender);

    // All three must have consumed exactly the production
    // cost from their input hopper.
    assert_eq!(
        worker_consumed, PRODUCTION_COST_PER_BOT,
        "Worker cycle cost"
    );
    assert_eq!(
        hauler_consumed, PRODUCTION_COST_PER_BOT,
        "Hauler cycle cost"
    );
    assert_eq!(
        defender_consumed, PRODUCTION_COST_PER_BOT,
        "Defender cycle cost"
    );
    // And explicitly: the three consumed amounts are equal
    // -- the cost is shared.
    assert_eq!(worker_consumed, hauler_consumed);
    assert_eq!(hauler_consumed, defender_consumed);
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
            ratio.set_weight(target, 1);
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
        // Issue #38 / ADR-0004: production-spawned
        // nanobots are top-level entities, not children
        // of the swarm. Count top-level entities whose
        // `SwarmMember` matches the swarm's `SwarmId`
        // and whose type is `target`.
        let swarm_id = app
            .world()
            .entity(swarm)
            .get::<SwarmId>()
            .copied()
            .expect("swarm must carry a SwarmId");
        let mut count = 0;
        {
            let world = app.world_mut();
            let mut query = world.query::<(
                &top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType,
                &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
            )>();
            for (ty, member) in query.iter(world) {
                if member.0 == swarm_id && *ty == target {
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
        intent::{IntentGrid, IntentKind},
        nanobot::{
            DEFAULT_PLANNED_WORK_TICKS, OwnerSwarm, PlannedKind, PlannedProductionTarget,
            PlannedStructure, SwarmId, completed_visual_color, planned_visual_color,
        },
    };
    let mut app = common::sim_app_with_production_planned();
    app.insert_resource(ProductionRatio::new());
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.set_weight(NanobotType::Worker, 10);
        ratio.set_weight(NanobotType::Hauler, 10);
        ratio.set_weight(NanobotType::Defender, 10);
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
        assert!(grid.paint_owned(IVec2::new(1, 0), IntentKind::Build, Some(SwarmId::PLAYER),));
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
    let center = app
        .world()
        .entity(planned_entity)
        .get::<Transform>()
        .expect("planned facility has transform")
        .translation
        .truncate();
    // The Worker must be placed AT the planned cell
    // center so the claim + arrive + work chain can
    // fire without a long walk. The build then
    // completes in `DEFAULT_PLANNED_WORK_TICKS` ticks
    // of worker time.
    let worker = common::spawn_worker_at(&mut app, center);

    // 1 tick for claim + arrive (worker is already at
    // the cell, so the arrive system fires on the same
    // tick as the claim), then `DEFAULT_PLANNED_WORK_TICKS`
    // ticks of work. The build completes on the
    // `DEFAULT_PLANNED_WORK_TICKS + 1`-th tick. We do
    // Completion must remain idle until logistics pays a full cycle.
    let build_ticks = 3 + DEFAULT_PLANNED_WORK_TICKS as usize;
    for _ in 0..(build_ticks + 200) {
        app.update();
        if app
            .world()
            .entity(planned_entity)
            .get::<PlannedStructure>()
            .is_none()
        {
            break;
        }
    }

    let world = app.world_mut();
    // The planned structure has been promoted: the
    // PlannedStructure component is gone, the entity now
    // carries a ProductionFacility + local Stockpile,
    // and the visual flipped to the completed color.
    let remaining = world
        .entity(planned_entity)
        .get::<PlannedStructure>()
        .copied();
    let worker_state = world.entity(worker);
    assert!(
        remaining.is_none(),
        "PlannedStructure must complete; remaining={remaining:?}, claim={}, progress={}, lease={}, movement={}",
        worker_state
            .contains::<top_down_2d_rts_prototype_nano_swarm::nanobot::PlannedStructureClaim>(),
        worker_state
            .contains::<top_down_2d_rts_prototype_nano_swarm::nanobot::PlannedStructureProgress>(),
        worker_state.contains::<top_down_2d_rts_prototype_nano_swarm::nanobot::RegionalLease>(),
        worker_state
            .contains::<top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent>(),
    );
    let facility = world
        .entity(planned_entity)
        .get::<ProductionFacility>()
        .expect("completion must replace PlannedStructure with a ProductionFacility");
    assert!(
        !facility.is_busy(),
        "completed facility must wait for delivered input before picking a target",
    );
    assert!(
        world
            .entity(planned_entity)
            .get::<PlannedProductionTarget>()
            .is_none(),
        "planning target sidecar must be removed at completion",
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
        ratio.set_weight(NanobotType::Worker, 10);
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
        ratio.set_weight(NanobotType::Worker, 1);
        ratio.set_weight(NanobotType::Hauler, 1);
    }
    let facility_pos = Vec2::new(0.0, 0.0);
    let facility_entity = {
        let mut f = ProductionFacility::new();
        f.blocked_types.insert(NanobotType::Worker);
        app.world_mut()
            .spawn((f, Transform::from_translation(facility_pos.extend(0.0))))
            .id()
    };
    common::fill_facility_input(&mut app, facility_entity);
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
        ratio.set_weight(NanobotType::Worker, 5);
        ratio.set_weight(NanobotType::Hauler, 5);
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
        ratio.set_weight(NanobotType::Hauler, 1);
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

#[test]
fn cold_start_facility_recovers_after_hopper_fills() {
    // Regression: a facility that starts with an empty hopper
    // and only one positive-deficit type used to deadlock. The
    // pick system blocked the sole deficit type (Defender)
    // because the hopper was below the production cost, and the
    // blocked set was only cleared at the end of a production
    // cycle -- which can never start while that type stays
    // blocked. Once haulers fill the hopper, production must
    // recover and produce the deficit type.
    //
    // This is the default scenario's exact cold-start shape:
    // seed swarm 4 W / 2 H / 0 D against the 60/30/10 default
    // ratio. Only Defender has a positive proportional deficit,
    // so it is the type that gets blocked and must later be
    // unblocked. Existing tests never hit this path because
    // `spawn_idle_facility_at` pre-fills the hopper; this test
    // spawns a bare `ProductionFacility::new()` (empty hopper),
    // runs empty-hopper ticks, then fills it.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 4), (NanobotType::Hauler, 2)],
    );
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        // 60/30/10 default mix: against 4 W / 2 H / 0 D only
        // Defender has a positive proportional deficit.
        ratio.set_weight(NanobotType::Worker, 6);
        ratio.set_weight(NanobotType::Hauler, 3);
        ratio.set_weight(NanobotType::Defender, 1);
    }
    // Facility with an EMPTY hopper, exactly as the default
    // scenario spawns it (ProductionFacility::new()).
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            Transform::from_translation(Vec2::new(0.0, 0.0).extend(0.0)),
        ))
        .id();

    // Drive several ticks with an empty hopper. Each tick the
    // pick system blocks Defender and -- with the fix -- drops
    // the blocked set again so it never deadlocks. With the
    // bug present, the blocked set would grow sticky here.
    for _ in 0..10 {
        app.update();
    }

    // The facility is still idle: no cycle can start with an
    // empty hopper. The load-bearing assertion is that the
    // blocked set is empty, not stuck on Defender.
    let facility_state = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .expect("facility must exist");
    assert_eq!(
        facility_state.current_target, None,
        "facility with an empty hopper must stay idle"
    );
    assert!(
        facility_state.blocked_types.is_empty(),
        "blocked set must not persist across empty-hopper ticks; otherwise the cold-start facility deadlocks"
    );

    // Haulers (logistics leg 3) now fill the hopper. This is
    // the moment the deadlock used to bite: Defender was stuck
    // in the blocked set from the empty-hopper ticks, so the
    // now-full hopper never produced anything.
    common::fill_facility_input(&mut app, facility);

    // 1 pick tick + PRODUCTION_TICKS_PER_BOT work ticks to
    // complete a cycle. The facility must pick Defender (the
    // sole positive-deficit type) and spawn one.
    for _ in 0..(1 + PRODUCTION_TICKS_PER_BOT as usize) {
        app.update();
    }

    assert_eq!(
        nanobot_count_by_type(app.world_mut(), NanobotType::Defender),
        1,
        "cold-start facility must recover once the hopper fills and produce the deficit Defender"
    );
}

#[test]
fn production_consumption_is_isolated_between_opponent_and_player_ledgers() {
    let mut app = App::new();
    let mut ratio = ProductionRatio::new();
    ratio.set_weight(NanobotType::Worker, 1);
    app.insert_resource(ratio)
        .init_resource::<ResourceLedger>()
        .add_systems(Update, production_facility_pick_target_system);
    let player = app.world_mut().spawn(SwarmBundle::default()).id();
    let opponent_id = SwarmId(2);
    let opponent = app
        .world_mut()
        .spawn(SwarmBundle {
            swarm_id: opponent_id,
            ..default()
        })
        .id();
    for owner in [player, opponent] {
        let mut facility = ProductionFacility::new();
        facility.input_amount = PRODUCTION_COST_PER_BOT;
        app.world_mut().spawn((facility, OwnerSwarm(owner)));
    }
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        opponent_id,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );

    app.update();

    let ledger = app.world().resource::<ResourceLedger>();
    assert_eq!(
        ledger.total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        0,
        "player cycle consumes only player material",
    );
    assert_eq!(
        ledger.total_for(opponent_id, ResourceKind::Minerals),
        0,
        "opponent cycle consumes only opponent material",
    );
}
