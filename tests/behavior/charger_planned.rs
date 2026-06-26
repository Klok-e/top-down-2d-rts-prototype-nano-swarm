//! Integration tests for issue #28: Migrate Chargers to
//! Planned Structures.
//!
//! Each test isolates one behaviour so a failure points at a
//! single contract:
//!
//!   1. Defend cell demand creates a Planned Charger
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
//!      completion; the plan promotes to a real Charger
//!      that uses the default `AUTO_CHARGER_INITIAL_AMOUNT`
//!      material so the existing charge sustain loop
//!      picks it up.
//!   6. A completed Charger provides charge resupply
//!      through the existing Charger behavior (rotation,
//!      refill, release).
//!   7. Charger logistics support through physical
//!      resources remains intact after completion: a
//!      Hauler can deliver minerals to a completed
//!      charger.
//!   8. The plan does not pile up across repeated demand
//!      ticks: the auto-creation system sees a planned
//!      Charger in the cell and does not spawn a second
//!      one.
//!   9. `PlannedKind::ALL` and `PlannedKind::COUNT`
//!      include the new Charger variant.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        completed_visual_color, planned_visual_color, Charge, Charger, ChargerAssignment,
        ChargerProgress, DefendHold, Health, OwnerSwarm, PlannedKind, PlannedStructure,
        PlannedStructureClaim, SwarmId, DEFAULT_PLANNED_WORK_TICKS, LOW_CHARGE_THRESHOLD,
        NANOBOT_DEFAULT_MAX_HEALTH,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_charge_planned()
}

fn paint_defend_owned(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
        Some(SwarmId::PLAYER),
    ));
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

fn place_defender_in_hold(app: &mut App, cell: IVec2) -> Entity {
    let cell_center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(app, cell_center);
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell });
    defender
}

#[test]
fn demand_creates_planned_charger_not_instant_charger() {
    // Acceptance: "Charger demand creates a Planned Charger
    // instead of an instant completed Charger." A Defend
    // cell with defender load produces a
    // `PlannedStructure` of `PlannedKind::Charger` after
    // one tick. No `Charger` entity exists yet.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let _defender = place_defender_in_hold(&mut app, cell);

    app.update();

    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "Defend cell demand must create a Planned Charger"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed Charger must exist before a Worker builds the plan"
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
    let _defender = place_defender_in_hold(&mut app, cell);

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
    // cell. Unowned paint falls back to the first swarm
    // in the world; player-painted cells produce
    // player-owned plans.
    let mut app = build_app();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let _defender = place_defender_in_hold(&mut app, cell);

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
fn no_planned_charger_without_demand() {
    // Sanity: a Defend-painted cell with no defender load
    // does NOT spawn a planned Charger. The "load" half
    // of the emergence contract still applies to the
    // planned path.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    // No defender is placed in hold; the demand is zero.

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
    // worker time, and the plan promotes to a real
    // `Charger` with the default shape
    // (`AUTO_CHARGER_INITIAL_AMOUNT` material on hand).
    // The visual flips to the completed color. The
    // `OwnerSwarm` is preserved through the promotion.
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
    // Default shape: AUTO_CHARGER_INITIAL_AMOUNT material
    // on hand so the rotation chain can pick the
    // completed charger up immediately.
    assert!(
        charger.has_supply(),
        "completed Charger must have AUTO_CHARGER_INITIAL_AMOUNT material on hand"
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
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let _swarm = common::spawn_swarm_at(&mut app, cell_center);
    let _plan = common::spawn_planned_charger_at_cell(&mut app, cell);
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
    // The completed Charger only has the default
    // `AUTO_CHARGER_INITIAL_AMOUNT` units. The defender
    // needs more material to recover from a
    // LOW_CHARGE_THRESHOLD start (the rotation chain
    // drains the buffer at 1 unit per charging tick).
    // Top up the buffer so the end-to-end test can
    // observe the full charge / release / return-to-hold
    // cycle. The existing charger-logistics tests use the
    // same pattern: pre-stock the charger rather than
    // depending on a hauler delivery mid-rotation.
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

    // Drive enough ticks for: rotation (1) + arrival
    // (~1) + refill (~18 ticks to go from LOW threshold
    // to MAX at (REFILL - DRAIN) per tick) + release (1).
    // 50 is a safe margin.
    for _ in 0..50 {
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
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell_center = common::cell_world_center(cell);
    let _plan = common::spawn_planned_charger_at_cell(&mut app, cell);
    let _worker = common::spawn_worker_at(&mut app, cell_center);
    // Leg source: a source-role stockpile close to the charger
    // (deposits are worker-only under ADR-0005). The charger is
    // the terminal sink; its source may be any stockpile with
    // material.
    let source_pos = Vec2::new(120.0, 0.0);
    let _source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let _hauler = common::spawn_hauler_at(&mut app, source_pos);

    // Build the plan first.
    let build_ticks = 1 + DEFAULT_PLANNED_WORK_TICKS as usize + 1;
    for _ in 0..build_ticks {
        app.update();
    }

    // The hauler needs the defend intent to be present so
    // the charger is a "real" sink in the sink selection.
    // (The hauler system does not actually need Defend
    // paint; the charger is a sink by its buffer shape.
    // We paint Defend anyway so the charger would be
    // useful in this scenario.)
    paint_defend_owned(&mut app, cell);

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
    // The completed charger's amount has grown from the
    // initial AUTO_CHARGER_INITIAL_AMOUNT because a
    // hauler delivered minerals. We assert strictly
    // greater than the initial amount to confirm the
    // delivery happened, without pinning the exact
    // amount.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::AUTO_CHARGER_INITIAL_AMOUNT;
    assert!(
        amount > AUTO_CHARGER_INITIAL_AMOUNT,
        "hauler must have delivered minerals to the completed Charger; got {amount}"
    );
}

#[test]
fn plan_does_not_pile_under_repeated_demand_ticks() {
    // Robustness: even when demand stays high across many
    // ticks, the auto-creation system does not pile a
    // second plan in the same cell, and does not plan
    // elsewhere while the first plan is still pending.
    // The busyness count includes the planned charger, so
    // the system sees the cell as already covered.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let cell = IVec2::new(0, 0);
    paint_defend_owned(&mut app, cell);
    let _defender = place_defender_in_hold(&mut app, cell);

    for _ in 0..20 {
        app.update();
    }

    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "auto-creation must not pile multiple Planned Chargers in the same cell"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed Charger must appear without a Worker building the plan"
    );
}

#[test]
fn planned_kind_includes_charger() {
    // Pin the `PlannedKind::ALL` / `PlannedKind::COUNT`
    // contract: the new variant shows up in the
    // foundation's stable iteration list with a stable
    // index distinct from the Source Stockpile, Sink
    // Stockpile, and Production Facility variants.
    let kinds: Vec<PlannedKind> = PlannedKind::ALL.to_vec();
    assert_eq!(kinds.len(), PlannedKind::COUNT);
    assert!(kinds.contains(&PlannedKind::SourceStockpile));
    assert!(kinds.contains(&PlannedKind::SinkStockpile));
    assert!(kinds.contains(&PlannedKind::ProductionFacility));
    assert!(
        kinds.contains(&PlannedKind::Charger),
        "PlannedKind::ALL must include Charger"
    );
    let charger_index = PlannedKind::Charger.index();
    let source_index = PlannedKind::SourceStockpile.index();
    let sink_index = PlannedKind::SinkStockpile.index();
    let production_index = PlannedKind::ProductionFacility.index();
    assert_ne!(charger_index, source_index);
    assert_ne!(charger_index, sink_index);
    assert_ne!(charger_index, production_index);
}
