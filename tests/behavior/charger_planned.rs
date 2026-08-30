//! Integration tests for issue #28: Migrate Chargers to
//! Planned Structures.
//!
//! Each test isolates one behaviour so a failure points at a
//! single contract:
//!
//!   1. Unserved low Charge creates a Planned Charger
//!      instead of an instant completed Charger.
//!   2. The plan uses the planned visual color so the
//!      player can tell the structure is not yet built.
//!   3. The plan is owned by the swarm that painted the
//!      Defend cell, satisfying the "owned-space
//!      constraints suitable for defense support"
//!      half of the acceptance.
//!   4. A Worker claims the planned Charger; only one
//!      worker holds the claim.
//!   5. A Worker builds the planned Charger to
//!      completion; the plan promotes to an empty Charger
//!      without minting minerals.
//!   6. A completed Charger provides charge resupply
//!      through the existing Charger behavior (rotation,
//!      refill, release).
//!   7. Charger logistics support through physical
//!      resources remains intact after completion: a
//!      Hauler can deliver minerals to a completed
//!      charger.
//!   8. Pending capacity prevents duplicate plans across
//!      repeated demand ticks.
//!   9. `PlannedKind::ALL` and `PlannedKind::COUNT`
//!      include the new Charger variant.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        BUILDING_FOOTPRINT_PADDING, BUILDING_FOOTPRINT_RADIUS, Charge, ChargePlugin, Charger,
        ChargerAssignment, ChargerProgress, DEFAULT_PLANNED_WORK_TICKS, DefendHold, Health,
        LOW_CHARGE_THRESHOLD, MaintenancePlugin, NANOBOT_DEFAULT_MAX_HEALTH, OwnerSwarm,
        PlannedKind, PlannedStructure, PlannedStructureClaim, Swarm, SwarmId, SwarmMember,
        completed_visual_color, planned_visual_color,
    },
    resources::{ResourceKind, ResourceLedger},
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_charge_planned()
}

fn planning_app() -> App {
    let mut app = common::minimal_app();
    app.add_plugins(ChargePlugin);
    app
}

fn paint_defend_owned(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId::PLAYER),));
}

fn planned_charger_count(world: &mut World) -> usize {
    let mut q = world.query::<&PlannedStructure>();
    q.iter(world)
        .filter(|p| p.kind == PlannedKind::Charger)
        .count()
}

fn charger_count(world: &mut World) -> usize {
    let mut q = world.query::<&Charger>();
    q.iter(world).count()
}

#[test]
fn unassigned_low_charge_defender_creates_planned_charger() {
    let mut app = planning_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "an unassigned low-Charge Defender must create a Planned Charger"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed Charger must exist before a Worker builds the plan"
    );
}

#[test]
fn full_charge_defender_does_not_create_planned_charger() {
    let mut app = planning_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    paint_defend_owned(&mut app, cell);
    let _defender = common::spawn_defender_in_hold_at(&mut app, cell);

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "a full-Charge Defender must not create Charger capacity",
    );
}

#[test]
fn remote_available_charger_capacity_suppresses_plan() {
    let mut app = planning_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let defender_cell = IVec2::ZERO;
    let charger_cell = IVec2::new(3, 0);
    paint_defend_owned(&mut app, defender_cell);
    paint_defend_owned(&mut app, charger_cell);
    let defender = common::spawn_defender_in_hold_at(&mut app, defender_cell);
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    let charger = common::spawn_operational_charger_at(&mut app, charger_cell, 12);
    app.world_mut()
        .entity_mut(charger)
        .insert(OwnerSwarm(swarm));

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "available swarm-wide Charger capacity must suppress a new plan",
    );
}

#[test]
fn remote_pending_capacity_prevents_duplicate_plans_across_ticks() {
    let mut app = planning_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let defender_cell = IVec2::ZERO;
    let plan_cell = IVec2::new(3, 0);
    paint_defend_owned(&mut app, defender_cell);
    paint_defend_owned(&mut app, plan_cell);
    let defender = common::spawn_defender_in_hold_at(&mut app, defender_cell);
    app.world_mut()
        .entity_mut(defender)
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    let plan = common::spawn_planned_charger_at_cell(&mut app, plan_cell);
    app.world_mut().entity_mut(plan).insert(OwnerSwarm(swarm));

    for _ in 0..5 {
        app.update();
    }

    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "pending swarm-wide capacity must remain idempotent across fixed steps",
    );
}

#[test]
fn fourth_low_charge_defender_exceeds_one_pending_chargers_capacity() {
    let mut app = planning_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    paint_defend_owned(&mut app, cell);
    let plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    app.world_mut().entity_mut(plan).insert(OwnerSwarm(swarm));
    for _ in 0..4 {
        let defender = common::spawn_defender_at(&mut app, common::cell_world_center(cell));
        app.world_mut()
            .entity_mut(defender)
            .get_mut::<Charge>()
            .expect("Defender has Charge")
            .current = LOW_CHARGE_THRESHOLD;
    }

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        2,
        "one pending Charger covers three low-Charge Defenders, not four",
    );
}

#[test]
fn plan_uses_nearest_non_overlapping_owned_defend_site() {
    let mut app = planning_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let legacy_hold_cell = IVec2::ZERO;
    let nearest_cell = IVec2::new(3, 0);
    paint_defend_owned(&mut app, legacy_hold_cell);
    paint_defend_owned(&mut app, nearest_cell);
    let defender = common::spawn_defender_in_hold_at(&mut app, legacy_hold_cell);
    let nearest_center = common::cell_world_center(nearest_cell);
    app.world_mut()
        .entity_mut(defender)
        .insert(Transform::from_translation(nearest_center.extend(0.0)))
        .get_mut::<Charge>()
        .expect("Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    let deposit_radius = 32.0;
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: nearest_center,
            amount: 20,
            capacity: 20,
            radius: deposit_radius,
        },
    );

    app.update();

    let world = app.world_mut();
    let (planned, transform) = world
        .query::<(&PlannedStructure, &Transform)>()
        .iter(world)
        .find(|(planned, _)| planned.kind == PlannedKind::Charger)
        .expect("low-Charge Defender creates a Charger plan");
    assert_eq!(
        planned.cell, nearest_cell,
        "physical Defender proximity, not a legacy hold, chooses the Defend cell",
    );
    assert!(
        transform.translation.truncate().distance(nearest_center)
            >= deposit_radius + BUILDING_FOOTPRINT_RADIUS + BUILDING_FOOTPRINT_PADDING,
        "planned Charger must not overlap the Resource Deposit",
    );
}

#[test]
fn newly_planned_charger_waits_for_next_regional_allocation_pass() {
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::ZERO;
    paint_defend_owned(&mut app, cell);
    let _defender = common::spawn_low_charge_defender_in_hold_at(&mut app, cell);
    let worker = common::spawn_worker_at(&mut app, common::cell_world_center(cell));

    // Charger demand runs after the current allocation acquisition, so the
    // newly created plan cannot be claimed until the next projection pass.
    app.update();
    let plan = {
        let world = app.world_mut();
        world
            .query::<(Entity, &PlannedStructure)>()
            .iter(world)
            .find_map(|(entity, planned)| (planned.kind == PlannedKind::Charger).then_some(entity))
            .expect("unserved low Charge must create a planned Charger")
    };
    assert!(
        app.world()
            .entity(worker)
            .get::<PlannedStructureClaim>()
            .is_none(),
        "the new plan must not be claimed during the creation tick"
    );

    app.update();

    assert_eq!(
        app.world()
            .entity(plan)
            .get::<PlannedStructure>()
            .unwrap()
            .active_worker,
        Some(worker),
        "regional acquisition must claim the plan on the next pass"
    );
    assert_eq!(
        app.world()
            .entity(worker)
            .get::<PlannedStructureClaim>()
            .unwrap()
            .target,
        plan,
    );
}

#[test]
fn planned_charger_uses_planned_visual_color() {
    // Acceptance: "Planned Structures are visibly distinct
    // from completed structures" (issue #21's visual
    // contract carries over to the new Charger kind). A
    // freshly planned Charger must use the planned visual
    // color so the player can tell it is not yet built.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let _defender = common::spawn_low_charge_defender_in_hold_at(&mut app, cell);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Sprite)>();
    let (planned, sprite) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::Charger)
        .expect("Planned Charger must exist");
    assert_eq!(planned.cell, cell);
    assert_eq!(
        sprite.color,
        planned_visual_color(),
        "Planned Charger must use the planned visual color"
    );
}

#[test]
fn planned_charger_is_owned_by_swarm_that_painted_defend_cell() {
    // Acceptance: "Planned Charger placement follows
    // owned-space constraints suitable for defense
    // support." The plan is stamped with the
    // `OwnerSwarm` of the swarm that painted the Defend
    // cell. Player-painted cells produce player-owned plans.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let _defender = common::spawn_low_charge_defender_in_hold_at(&mut app, cell);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &OwnerSwarm)>();
    let (planned, owner) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::Charger)
        .expect("Planned Charger must exist");
    assert_eq!(planned.kind, PlannedKind::Charger);
    assert_eq!(
        owner.0, swarm,
        "Planned Charger must be owned by the swarm that painted the Defend cell"
    );
}

#[test]
fn each_swarm_plans_only_in_its_owned_defend_paint() {
    let mut app = planning_app();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let opponent_id = SwarmId(11);
    let opponent = app
        .world_mut()
        .spawn((Swarm {}, opponent_id, Transform::default()))
        .id();
    let player_cell = IVec2::new(-1, 0);
    let opponent_cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(player_cell, IntentKind::Defend, Some(SwarmId::PLAYER));
        grid.paint_owned(opponent_cell, IntentKind::Defend, Some(opponent_id));
    }
    let player_defender =
        common::spawn_defender_at(&mut app, common::cell_world_center(player_cell));
    app.world_mut()
        .entity_mut(player_defender)
        .get_mut::<Charge>()
        .expect("player Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;
    let opponent_defender =
        common::spawn_defender_at(&mut app, common::cell_world_center(opponent_cell));
    app.world_mut()
        .entity_mut(opponent_defender)
        .insert(SwarmMember::new(opponent_id));
    app.world_mut()
        .entity_mut(opponent_defender)
        .get_mut::<Charge>()
        .expect("opponent Defender has Charge")
        .current = LOW_CHARGE_THRESHOLD;

    app.update();

    let world = app.world_mut();
    let owners = world
        .query::<(&PlannedStructure, &OwnerSwarm)>()
        .iter(world)
        .filter_map(|(planned, owner)| (planned.kind == PlannedKind::Charger).then_some(owner.0))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(owners, std::collections::HashSet::from([player, opponent]));
}

#[test]
fn no_planned_charger_without_demand() {
    // Defend paint alone does not create Charger service need.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    // No low-Charge Defender exists.

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "no Planned Charger must emerge without defender demand"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no Charger must emerge without defender demand"
    );
}

#[test]
fn idle_worker_claims_planned_charger() {
    // Acceptance: "One Worker builds the Planned Charger
    // to completion." The "claim" half of the contract:
    // an idle Worker at the planned cell receives a
    // `PlannedStructureClaim` aimed at the plan, and the
    // plan records the worker as its `active_worker`.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    let worker = common::spawn_worker_at(&mut app, cell_center);

    app.update();

    let world = app.world();
    let claim = world
        .entity(worker)
        .get::<PlannedStructureClaim>()
        .expect("idle worker must claim the planned Charger");
    assert_eq!(claim.target, plan);
    let planned = world.entity(plan).get::<PlannedStructure>().unwrap();
    assert_eq!(
        planned.active_worker,
        Some(worker),
        "planned Charger must record the worker as active_worker"
    );
}

#[test]
fn only_one_worker_claims_a_planned_charger() {
    // "Other Workers do not work on an already claimed
    // Planned Structure" (issue #21's reservation
    // contract carries over to the new Charger kind).
    // Two idle workers, one planned Charger: exactly one
    // worker holds the claim.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    let worker_a = common::spawn_worker_at(&mut app, cell_center);
    let worker_b = common::spawn_worker_at(&mut app, cell_center);

    app.update();

    let world = app.world();
    let planned = world.entity(plan).get::<PlannedStructure>().unwrap();
    let active = planned
        .active_worker
        .expect("planned Charger must be claimed");
    assert!(
        active == worker_a || active == worker_b,
        "active worker must be one of the two idle workers"
    );
    let claim_count = (world
        .entity(worker_a)
        .get::<PlannedStructureClaim>()
        .is_some() as u32)
        + (world
            .entity(worker_b)
            .get::<PlannedStructureClaim>()
            .is_some() as u32);
    assert_eq!(
        claim_count, 1,
        "exactly one worker must hold the planned Charger claim; got {claim_count}"
    );
}

#[test]
fn worker_builds_planned_charger_to_completion() {
    // Acceptance: "One Worker builds the Planned Charger
    // to completion." A Worker at the cell claims the
    // plan, spends `DEFAULT_PLANNED_WORK_TICKS` ticks of
    // worker time, and the plan promotes to an empty
    // `Charger`. The visual flips to the completed color.
    // `OwnerSwarm` remains through promotion.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, cell_center);
    let plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    // OwnerSwarm is normally stamped by the auto-creation
    // system; for this test-driven flow we stamp it
    // ourselves so the promotion path's preservation
    // contract is exercised.
    app.world_mut().entity_mut(plan).insert(OwnerSwarm(swarm));
    let _worker = common::spawn_worker_at(&mut app, cell_center);
    let ledger_before = app
        .world()
        .resource::<ResourceLedger>()
        .total_for(SwarmId::PLAYER, ResourceKind::Minerals);

    // 1 tick for claim + arrive (worker is at the cell so
    // arrive fires on the same tick as claim), then
    // DEFAULT_PLANNED_WORK_TICKS ticks of work, then the
    // promotion tick. We add 1 buffer tick for safety.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 1;
    for _ in 0..build_ticks {
        app.update();
    }

    let world = app.world();
    // The plan is gone, replaced by a real Charger.
    assert!(
        world.entity(plan).get::<PlannedStructure>().is_none(),
        "PlannedStructure must be removed on completion"
    );
    let charger = world
        .entity(plan)
        .get::<Charger>()
        .expect("completion must replace PlannedStructure with a Charger");
    assert_eq!(
        charger.cell, cell,
        "completed Charger must record the plan's cell"
    );
    assert_eq!(charger.amount, 0, "completed Charger must start empty");
    assert!(
        !charger.has_supply(),
        "empty completed Charger cannot supply charge"
    );
    assert_eq!(
        world
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        ledger_before,
        "Charger completion must not seed the resource ledger",
    );
    // Visual flipped to the completed color.
    let sprite = world
        .entity(plan)
        .get::<Sprite>()
        .expect("completed Charger must carry a Sprite");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed Charger must use the completed visual color"
    );
    // OwnerSwarm is preserved through the promotion so
    // the completed charger keeps the swarm that painted
    // the Defend cell.
    let owner = world
        .entity(plan)
        .get::<OwnerSwarm>()
        .expect("OwnerSwarm must be preserved on Charger promotion");
    assert_eq!(
        owner.0, swarm,
        "completed Charger must keep the plan's OwnerSwarm"
    );
}

#[test]
fn completed_planned_charger_provides_charge_to_defenders() {
    // Acceptance: "Completed Chargers provide charge
    // resupply through existing Charger behavior." A
    // Planned Charger is built by a Worker, then a
    // low-charge defender in a Defend-hold on the same
    // cell rotates to the completed Charger, refills its
    // charge, and returns to hold. The end-to-end
    // sustain loop runs through the existing charge
    // systems (rotation, arrive, work) without any new
    // wiring.
    let mut app = build_app();
    app.add_plugins(MaintenancePlugin);
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let swarm = common::spawn_swarm_at(&mut app, cell_center);
    let plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    app.world_mut().entity_mut(plan).insert(OwnerSwarm(swarm));
    let _worker = common::spawn_worker_at(&mut app, cell_center);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    // Paint the Defend cell so the hold system keeps the
    // defender in hold after the rotation chain releases
    // them. The test-driven flow (spawn plan / build /
    // charge) bypasses the auto-creation system's paint,
    // so the test paints the cell directly.
    paint_defend_owned(&mut app, cell);

    // Build the plan first. 1 tick claim+arrive,
    // DEFAULT_PLANNED_WORK_TICKS ticks of work, +1 for
    // the promotion tick.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 1;
    for _ in 0..build_ticks {
        app.update();
    }
    // Completed chargers begin empty. Top up this focused charge-loop
    // fixture directly; logistics delivery is covered below.
    {
        let w = &mut app.world_mut();
        let mut q = w.query::<&mut Charger>();
        let mut charger = q.single_mut(w).expect("completed Charger must exist");
        charger.amount = charger.capacity;
    }
    // The charger is now a real Charger. Find its entity
    // by querying for `(Entity, &Charger)` so the query
    // state yields the entity handle directly.
    let charger_entity = {
        let world = app.world_mut();
        let mut q = world.query::<(Entity, &Charger)>();
        q.iter(world)
            .next()
            .map(|(e, _)| e)
            .expect("completed Charger entity must exist")
    };

    // Put the defender into a low-charge state and into
    // a DefendHold so the rotation chain picks it up.
    {
        let w = &mut app.world_mut();
        let mut entity = w.entity_mut(defender);
        let mut c = entity.get_mut::<Charge>().expect("defender has Charge");
        c.current = LOW_CHARGE_THRESHOLD;
        entity.insert(DefendHold { cell });
    }

    // Drive enough ticks for rotation, 19 supplied pulses, and
    // re-entry through current Defend allocation.
    for _ in 0..300 {
        app.update();
    }

    // The defender is back in DefendHold with a charge
    // above the rotation threshold (the rotation chain
    // released them after the refill).
    let world = app.world();
    assert!(
        world.entity(defender).get::<DefendHold>().is_some(),
        "defender must return to DefendHold after charging from a completed Charger"
    );
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "ChargerAssignment must be cleared after charging"
    );
    assert!(
        world.entity(defender).get::<ChargerProgress>().is_none(),
        "ChargerProgress must be cleared after charging"
    );
    let charge = world
        .entity(defender)
        .get::<Charge>()
        .expect("defender still has Charge")
        .current;
    assert!(
        charge >= LOW_CHARGE_THRESHOLD,
        "defender's charge must be at or above the rotation threshold after charging; got {charge}"
    );
    // The defender must reach the completed Charger before
    // the empty-charge health loss collapses them. The
    // health-loss system is gated on ChargerAssignment /
    // ChargerProgress absence, so a charging defender is
    // safe; we still assert full health to pin the
    // "planned-then-built charger behaves identically to a
    // directly-seeded one" cross-check.
    let health = world
        .entity(defender)
        .get::<Health>()
        .expect("defender still alive")
        .current;
    assert_eq!(
        health, NANOBOT_DEFAULT_MAX_HEALTH,
        "defender must reach the completed Charger before health loss collapses them"
    );
    // The charger has been drained by the per-tick rate
    // while the defender was charging. The exact amount is
    // not pinned (it depends on how long the defender spent
    // charging), but the buffer must not be empty.
    let c = world.entity(charger_entity).get::<Charger>().unwrap();
    assert!(c.has_supply(), "charger must still have material on hand");
}

#[test]
fn hauler_delivers_to_completed_planned_charger() {
    // Acceptance: "Charger logistics support through
    // physical resources remains intact after
    // completion." A Hauler can deliver minerals to a
    // completed Charger that was originally planned and
    // built by a Worker. The logistics chain does not
    // care whether the charger was seeded directly or
    // promoted from a plan.
    let mut app = build_app();
    let cell = IVec2::new(2, 0);
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell_center = common::cell_world_center(cell);
    let plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    app.world_mut().entity_mut(plan).insert(OwnerSwarm(swarm));
    let _worker = common::spawn_worker_at(&mut app, cell_center);
    // Terminal legs source only from same-swarm Sink stockpiles.
    let source_pos = Vec2::new(120.0, 0.0);
    let source = common::spawn_sink_stockpile(&mut app, source_pos, 1000, 1000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    let _hauler = common::spawn_hauler_at(&mut app, source_pos);

    // Build the plan first.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 1;
    for _ in 0..build_ticks {
        app.update();
    }

    // Drive enough ticks for the hauler to walk to the
    // deposit, load, walk to the charger, and deliver.
    // The distance is ~120 world units; at bot_speed
    // 5.0 the hauler needs ~25 ticks for the one-way
    // trip. 500 is a safe margin.
    for _ in 0..500 {
        app.update();
    }

    let world = app.world_mut();
    let amount = {
        let mut q = world.query::<&Charger>();
        let charger = q.iter(world).next().expect("completed Charger must exist");
        charger.amount
    };
    assert!(
        amount > 0,
        "hauler must deliver minerals to initially empty completed Charger; got {amount}"
    );
}

#[test]
fn plan_does_not_pile_under_repeated_demand_ticks() {
    // Pending capacity remains reserved across repeated demand ticks.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let _defender = common::spawn_low_charge_defender_in_hold_at(&mut app, cell);

    for _ in 0..20 {
        app.update();
    }

    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "auto-creation must not pile plans while pending capacity is sufficient"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed Charger must appear without a Worker building the plan"
    );
}
