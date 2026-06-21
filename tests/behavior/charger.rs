//! Integration tests for issue #14: Chargers and defender
//! Charge sustain loop.
//!
//! Each test isolates one behavior so a failure points at a
//! single contract: charger auto-emergence from Defend Zone
//! load, charger emergence respecting existing busyness,
//! logistics dependence (a charger without material is not a
//! working rotation target), weakening of attack/defense on
//! low charge, health loss on empty/ignored charge, and the
//! automatic rotation of defenders to working chargers.
//!
//! The pure-helper unit tests (charge helpers, multipliers,
//! Charger data) live in `src/nanobot/charge.rs`.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        Charge, Charger, ChargerAssignment, ChargerProgress, DefendAssignment, DefendHold, Health,
        Nanobot, PlannedKind, PlannedStructure, SoftWorkSlots, CHARGE_DRAIN_PER_TICK,
        CHARGE_REFILL_PER_TICK, DEFENDER_BASE_ATTACK, DEFENDER_BASE_DEFENSE,
        EMPTY_CHARGE_HEALTH_LOSS_PER_TICK, LOW_CHARGE_THRESHOLD, MAX_CHARGE,
        NANOBOT_DEFAULT_MAX_HEALTH, WEAKENED_CHARGE_THRESHOLD,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    // Issue #28: Chargers emerge as Planned Structures and
    // are built by a Worker. The behaviour tests that
    // exercise the demand side need the planned-structure
    // plugin loaded so the auto-creation system can spawn
    // a plan and a Worker can build it. The
    // `sim_app_with_charge_planned` seam bundles both
    // plugins in the order the production code wires them.
    common::sim_app_with_charge_planned()
}

fn charger_count(world: &mut World) -> usize {
    let mut q = world.query::<&Charger>();
    q.iter(world).count()
}

fn planned_charger_count(world: &mut World) -> usize {
    let mut q = world.query::<&PlannedStructure>();
    q.iter(world)
        .filter(|p| p.kind == PlannedKind::Charger)
        .count()
}

fn read_charge(app: &App, defender: Entity) -> Option<f32> {
    app.world()
        .entity(defender)
        .get::<Charge>()
        .map(|c| c.current)
}

fn read_health(app: &App, defender: Entity) -> Option<u32> {
    app.world()
        .entity(defender)
        .get::<Health>()
        .map(|h| h.current)
}

#[test]
fn charger_auto_emerges_in_defend_cell_with_defender_load() {
    // Acceptance: "Chargers emerge from Defend Zone load..."
    // As of issue #28, demand creates a Planned Charger
    // (not a completed Charger). A Defend-painted cell
    // with a holding defender must gain a Planned Charger
    // on the next tick. The plan lives in the cell so the
    // player can see the support structure co-located with
    // the defense; the completed Charger only appears
    // after a Worker builds the plan.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    // Spawn the defender and then plant it in a DefendHold
    // below; the auto-creation system reads "holding or
    // assigned" so a bare idle defender would not show up
    // in the load count.
    common::spawn_defender_at(&mut app, cell_center);

    // Pre-condition: zero chargers and zero planned
    // chargers.
    assert_eq!(charger_count(app.world_mut()), 0);
    assert_eq!(planned_charger_count(app.world_mut()), 0);

    // Place a defender into hold on the same cell so the
    // auto-creation system sees load.
    {
        let w = app.world_mut();
        let entity = w
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(w)
            .next()
            .expect("defender was just spawned");
        w.entity_mut(entity).insert(DefendHold { cell });
    }

    app.update();

    // Demand created a planned charger; the completed
    // Charger does NOT exist yet (a Worker must build the
    // plan first).
    assert_eq!(
        planned_charger_count(app.world_mut()),
        1,
        "one planned charger must emerge from a Defend cell with a holding defender"
    );
    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no completed charger must exist before a Worker builds the plan"
    );
    // The plan is in the painted cell and at the cell's
    // world center.
    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let (planned, transform) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::Charger)
        .expect("Planned Charger exists");
    assert_eq!(planned.cell, cell);
    assert!(
        (transform.translation.truncate() - cell_center).length() < 1.0,
        "Planned Charger must be at the cell's world center"
    );
}

#[test]
fn charger_does_not_emerge_in_cell_without_load() {
    // Sanity: a Defend cell with no defenders must not
    // spawn a charger (planned or completed). The "load"
    // half of the emergence contract requires at least one
    // defender committed to the cell.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );

    app.update();

    assert_eq!(
        charger_count(app.world_mut()),
        0,
        "no charger without a holding defender"
    );
    assert_eq!(
        planned_charger_count(app.world_mut()),
        0,
        "no planned charger without a holding defender"
    );
}

#[test]
fn charger_emergence_respects_existing_charger_busyness() {
    // Acceptance: "Chargers emerge from Defend Zone load AND
    // existing charger busyness." A cell with one charger
    // and many defenders must spawn additional chargers; a
    // cell with one charger and few defenders must not.
    //
    // The MAX_DEFENDERS_PER_CHARGER threshold drives the
    // emergence: 1 charger covers up to 3 defenders; 4+
    // defenders ask for a second charger. The test plants
    // 5 holding defenders in one cell, then asserts that
    // a second (planned) charger appears and the first
    // charger is still there (the existing one is not
    // destroyed).
    //
    // As of issue #28 the additional charger emerges as a
    // Planned Charger (the demand path produces a plan, a
    // Worker builds it, the completed Charger takes over).
    // The busyness count includes BOTH completed Chargers
    // AND Planned Chargers in the same cell so the
    // auto-creation loop does not pile plans.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);

    // Plant 5 holding defenders in the same cell. The cell
    // already has 1 charger; with MAX_DEFENDERS_PER_CHARGER
    // = 3 the demand is 2 chargers.
    for i in 0..5 {
        // Spread them out a tiny bit so separation forces
        // do not pile them on top of each other. The cell
        // is much larger than a bot radius so a few-pixel
        // jitter is invisible to the cell-classification
        // step.
        let jitter = (i as f32 - 2.0) * 2.0;
        let d = common::spawn_defender_at(&mut app, cell_center + Vec2::new(jitter, 0.0));
        app.world_mut().entity_mut(d).insert(DefendHold { cell });
    }

    app.update();

    let completed = charger_count(app.world_mut());
    let planned = planned_charger_count(app.world_mut());
    let total = completed + planned;
    assert!(
        total >= 2,
        "busy cell must spawn an additional charger (completed={completed}, planned={planned})"
    );
    // The completed charger is still there (not destroyed).
    assert_eq!(
        completed, 1,
        "pre-existing completed charger must not be destroyed by the demand loop"
    );
    // And the demand produced at least one more plan.
    assert!(
        planned >= 1,
        "busy cell must plan at least one additional charger; got {planned}"
    );
    // All chargers (planned and completed) are in the
    // same cell.
    let world = app.world_mut();
    let mut q = world.query::<&Charger>();
    for c in q.iter(world) {
        assert_eq!(c.cell, cell);
    }
    let mut qp = world.query::<&PlannedStructure>();
    for p in qp.iter(world).filter(|p| p.kind == PlannedKind::Charger) {
        assert_eq!(p.cell, cell);
    }
}

#[test]
fn charger_does_not_emerge_extra_when_load_below_busy_threshold() {
    // Companion to the busyness test: a cell with one charger
    // and fewer defenders than MAX_DEFENDERS_PER_CHARGER must
    // not spawn a second charger. The existing one is enough.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);
    // 1 charger and 2 defenders: 2 < MAX_DEFENDERS_PER_CHARGER
    // (3), so the demand is still 1 charger. No second one.
    for i in 0..2 {
        let jitter = (i as f32 - 0.5) * 4.0;
        let d = common::spawn_defender_at(&mut app, cell_center + Vec2::new(jitter, 0.0));
        app.world_mut().entity_mut(d).insert(DefendHold { cell });
    }

    app.update();

    assert_eq!(
        charger_count(app.world_mut()),
        1,
        "no extra charger when load is below the busyness threshold"
    );
}

#[test]
fn only_defenders_have_charge_component() {
    // Acceptance: "Only Defenders use Charge." The Charge
    // component is only inserted on Defenders, and the charge
    // systems filter on NanobotType::Defender. The test
    // spawns a Worker, a Hauler, and a Defender, asserts the
    // Charge component is only present on the Defender, and
    // runs a few ticks to verify the drain does not touch
    // the other types.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));

    let worker = common::spawn_worker_at(&mut app, Vec2::new(0.0, 0.0));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Pre-condition: only the defender has Charge.
    assert!(app.world().entity(worker).get::<Charge>().is_none());
    assert!(app.world().entity(hauler).get::<Charge>().is_none());
    assert!(app.world().entity(defender).get::<Charge>().is_some());

    for _ in 0..5 {
        app.update();
    }

    // Drain only affected the defender. Worker and Hauler
    // still have no Charge component.
    assert!(app.world().entity(worker).get::<Charge>().is_none());
    assert!(app.world().entity(hauler).get::<Charge>().is_none());
    let defender_charge = read_charge(&app, defender).expect("defender has Charge");
    // Defender drained 5 ticks. The exact value depends on
    // whether the rotation/auto-creation systems also fired
    // (they didn't because there is no Defend paint and no
    // Charger), so the post-condition is "strictly less than
    // MAX_CHARGE and non-negative".
    assert!(defender_charge < MAX_CHARGE);
    assert!(defender_charge > 0.0);
}

#[test]
fn low_charge_reduces_defender_attack_and_defense() {
    // Acceptance: "Low Charge reduces Defender attack/defense."
    // The pure helper returns the multiplier; the test
    // verifies the contract by calling the helper with
    // several charge values and checking the resulting
    // attack/defense.
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        charge_strength_multiplier, effective_attack, effective_defense,
    };

    // Full charge: full attack and full defense.
    assert!((effective_attack(MAX_CHARGE) - DEFENDER_BASE_ATTACK).abs() < 1e-5);
    assert!((effective_defense(MAX_CHARGE) - DEFENDER_BASE_DEFENSE).abs() < 1e-5);

    // At the weakened threshold: still full strength (the
    // helper is `>=` on the threshold).
    assert!((effective_attack(WEAKENED_CHARGE_THRESHOLD) - DEFENDER_BASE_ATTACK).abs() < 1e-5);
    assert!((effective_defense(WEAKENED_CHARGE_THRESHOLD) - DEFENDER_BASE_DEFENSE).abs() < 1e-5);

    // Below the weakened threshold: attack and defense scale
    // linearly. A charge of 0.1 (one third of the threshold)
    // yields a 1/3 multiplier.
    let third = WEAKENED_CHARGE_THRESHOLD / 3.0;
    let mult = charge_strength_multiplier(third);
    assert!((mult - 1.0 / 3.0).abs() < 1e-5);
    assert!((effective_attack(third) - DEFENDER_BASE_ATTACK * mult).abs() < 1e-5);
    assert!((effective_defense(third) - DEFENDER_BASE_DEFENSE * mult).abs() < 1e-5);

    // Empty charge: zero attack and zero defense.
    assert_eq!(effective_attack(0.0), 0.0);
    assert_eq!(effective_defense(0.0), 0.0);
}

#[test]
fn empty_charge_causes_defender_health_loss_when_no_charger() {
    // Acceptance: "Empty/ignored Charge causes Defender
    // health loss." A defender in DefendHold with empty
    // charge and no working charger reachable must lose
    // health per tick. The empty charge is the trigger; the
    // absence of a working charger is what makes it
    // "ignored" (no rotation happens).
    //
    // The test pre-spawns a charger with `amount = 0` so
    // the auto-creation system does not also spawn a working
    // charger (which would let the rotation chain absorb the
    // defender and stop the health loss). The pre-spawned
    // charger is "not working" because it has no supply, so
    // the rotation system does not pick it.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 0);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    // Empty out the charge and place the defender in hold.
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    let start_health = read_health(&app, defender).expect("defender has Health");
    assert_eq!(start_health, NANOBOT_DEFAULT_MAX_HEALTH);

    // Run a few ticks. The health must drop by at least one
    // tick of damage (the test allows a few extra ticks for
    // the drain system to have nothing to do; the empty
    // charge is the trigger, the loss is per tick).
    for _ in 0..5 {
        app.update();
    }

    let end_health = read_health(&app, defender).expect("defender still alive");
    let lost = start_health - end_health;
    assert!(
        lost > 0,
        "defender must lose health with empty charge; lost {lost}"
    );
    // Pin the per-tick rate: 5 ticks at the constant loss
    // rate. The system fires on the same tick as the drain,
    // so the math is `ticks * rate` exactly (the loss is
    // not gated on charge state after the first tick).
    assert_eq!(
        lost,
        EMPTY_CHARGE_HEALTH_LOSS_PER_TICK * 5,
        "health loss rate must be EMPTY_CHARGE_HEALTH_LOSS_PER_TICK per tick"
    );
}

#[test]
fn defender_does_not_lose_health_while_charging_at_a_working_charger() {
    // Companion: a defender that is at a working charger is
    // NOT in the "ignored" case, so the health loss system
    // must not fire for them. The test plants the defender
    // directly in the ChargerProgress state so the strict
    // assertion is not gated on rotation mechanics.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
        w.entity_mut(defender).insert(ChargerAssignment { charger });
        w.entity_mut(defender).insert(ChargerProgress { charger });
    }

    let start_health = read_health(&app, defender).expect("defender has Health");
    assert_eq!(start_health, NANOBOT_DEFAULT_MAX_HEALTH);

    for _ in 0..5 {
        app.update();
    }

    // Drain still fires, but health loss is gated on
    // ChargerAssignment / ChargerProgress absence, so the
    // defender at a working charger must not lose any
    // health.
    let end_health = read_health(&app, defender).expect("defender has Health");
    assert_eq!(
        end_health, NANOBOT_DEFAULT_MAX_HEALTH,
        "defender at a working charger must not lose health"
    );
}

#[test]
fn defender_rotates_to_working_charger_when_charge_is_low() {
    // Acceptance: "Defenders automatically rotate to working
    // chargers when low on Charge." A holding defender with
    // charge at or below LOW_CHARGE_THRESHOLD must receive a
    // ChargerAssignment aimed at a working charger; the
    // DefendHold marker is removed; the soft work slot is
    // released.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        // Charge at exactly the low threshold: must trigger rotation.
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    // The test directly inserts `DefendHold` rather than
    // going through the defend assignment system, so the
    // soft work slot is not occupied yet. The rotation
    // system is expected to release the slot (no-op here
    // because the slot count is 0) and route the defender
    // to the charger.

    app.update();

    // Post-rotation: the defender has a ChargerAssignment,
    // no DefendHold, and the slot is released.
    let world = app.world();
    let has_charger_assignment = world.entity(defender).get::<ChargerAssignment>().is_some();
    assert!(
        has_charger_assignment,
        "low-charge defender must be assigned to a charger"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_none(),
        "DefendHold must be removed when the defender rotates to a charger"
    );
    let slots = world.resource::<SoftWorkSlots>();
    assert_eq!(
        slots.occupied(cell, IntentKind::Defend),
        0,
        "soft work slot must be released when the defender leaves hold"
    );
    // The charger is the right one.
    let assignment = world.entity(defender).get::<ChargerAssignment>().unwrap();
    assert_eq!(assignment.charger, _charger);
}

#[test]
fn defender_does_not_rotate_to_empty_charger() {
    // Companion: a charger with no material is not a
    // "working" rotation target, so a low-charge defender
    // must not rotate to it. The defender stays in hold and
    // the empty-charge health loss system fires (covered by
    // another test).
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    // Charger with NO material: not a working rotation target.
    let _charger = common::spawn_charger_at(&mut app, cell, 0);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "defender must not rotate to an empty charger"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_some(),
        "defender must stay in hold when no working charger is available"
    );
}

#[test]
fn defender_charges_at_a_working_charger_and_returns_to_defend() {
    // End-to-end: a defender in DefendHold with low charge
    // rotates to a working charger, the work system refills
    // the charge, and once the charge is full the work
    // system releases the defender so the defend pool can
    // re-assign them. The final assertion is that the
    // defender is again in DefendHold after enough ticks.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 200);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.2;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    // Run enough ticks for: rotation (1) + travel (~1
    // tick at the same cell) + arrival (1) + charge refill
    // (~18 ticks to go from 0.2 to 1.0 at the (REFILL -
    // DRAIN) net rate) + re-assignment (1) + travel back
    // (~1 tick) + hold detection (1). 50 is a safe margin.
    for _ in 0..100 {
        app.update();
    }

    // The defender's charge is somewhere in the cycle:
    // either fully charged after a recent refill, or
    // between LOW_CHARGE_THRESHOLD and MAX_CHARGE while
    // holding the cell. The cycle is "hold -> drain ->
    // rotate to charger -> refill -> release -> hold".
    // The test pins the observable shape of the cycle: the
    // defender is in the defend hold (released from the
    // charger) and the charger has been drained by the
    // per-tick rate.
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "defender must return to DefendHold after charging"
    );
    let charge = read_charge(&app, defender).expect("defender has Charge");
    assert!(
        charge >= LOW_CHARGE_THRESHOLD,
        "defender in DefendHold must have charge above the rotation threshold; got {charge}"
    );
    assert!(
        charge <= MAX_CHARGE,
        "charge cannot exceed the cap; got {charge}"
    );
    // No leftover charger markers.
    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerAssignment>()
            .is_none(),
        "ChargerAssignment must be removed after charging"
    );
    assert!(
        app.world()
            .entity(defender)
            .get::<ChargerProgress>()
            .is_none(),
        "ChargerProgress must be removed after charging"
    );
    // The charger's material has been drained by the per-tick
    // rate. The exact amount is not pinned because the test
    // also allows for ticks where the defender was in
    // transit, but it must be strictly less than the
    // initial 200.
    let c = app.world().entity(_charger).get::<Charger>().unwrap();
    assert!(
        c.amount < 200,
        "charger material must be drained while defenders charge from it; got {}",
        c.amount
    );
}

#[test]
fn charger_requires_logistics_support_via_physical_resources() {
    // Acceptance: "Chargers require logistics support via
    // physical resources." A charger with no material is not
    // a working rotation target; a defender with low charge
    // and no working charger does not rotate and starts
    // losing health. The same scenario, with a stocked
    // charger, lets the defender rotate and recover.
    //
    // The test pins the "logistics support" contract end to
    // end: a defender at a held cell with an empty charger
    // in the same cell must end up in worse health than a
    // defender at the same cell with a stocked charger.
    let mut app_empty = build_app();
    {
        let _swarm = common::spawn_swarm_at(&mut app_empty, Vec2::new(0.0, 0.0));
        let cell = IVec2::new(0, 0);
        app_empty.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            PAINT_STRENGTH_CAP,
        );
        let cell_center = common::cell_world_center(cell);
        // Pre-spawn an empty charger so the auto-creation
        // system does not also create a working charger
        // (which would let the rotation chain absorb the
        // defender and stop the health loss). The empty
        // charger is "not working" because it has no supply.
        let _charger = common::spawn_charger_at(&mut app_empty, cell, 0);
        let defender = common::spawn_defender_at(&mut app_empty, cell_center);
        {
            let w = app_empty.world_mut();
            w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.0;
            w.entity_mut(defender).insert(DefendHold { cell });
        }
        for _ in 0..5 {
            app_empty.update();
        }
        let health_empty = read_health(&app_empty, defender);
        // The defender is collapsing; the entity may be
        // despawned if health reached zero. The "logistics
        // dependence" contract is "no material => no
        // recovery => health drops"; either the entity
        // exists with reduced health or it was despawned.
        if let Some(h) = health_empty {
            assert!(
                h < NANOBOT_DEFAULT_MAX_HEALTH,
                "empty charger must lead to health loss; got {h}"
            );
        }
    }

    let mut app_filled = build_app();
    {
        let _swarm = common::spawn_swarm_at(&mut app_filled, Vec2::new(0.0, 0.0));
        let cell = IVec2::new(0, 0);
        app_filled.world_mut().resource_mut::<IntentGrid>().paint(
            cell,
            IntentKind::Defend,
            PAINT_STRENGTH_CAP,
        );
        let cell_center = common::cell_world_center(cell);
        let _charger = common::spawn_charger_at(&mut app_filled, cell, 200);
        let defender = common::spawn_defender_at(&mut app_filled, cell_center);
        {
            let w = app_filled.world_mut();
            w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD;
            w.entity_mut(defender).insert(DefendHold { cell });
        }
        // Drive a few ticks: rotation + arrival + at least
        // one charging tick. 30 is a safe margin.
        for _ in 0..30 {
            app_filled.update();
        }
        let health_filled = read_health(&app_filled, defender)
            .expect("defender with a stocked charger must still be alive");
        assert_eq!(
            health_filled, NANOBOT_DEFAULT_MAX_HEALTH,
            "stocked charger must keep defender at full health; got {health_filled}"
        );
    }
}

#[test]
fn hauler_delivers_minerals_to_a_charger_with_free_space() {
    // The "logistics support" half of the contract is the
    // physical resource flow: haulers can deliver minerals
    // to a charger with free space. A stocked-up charger
    // (via hauler delivery) is what keeps a defended cell
    // supplied when the player is not actively painting
    // intent. The test plants a deposit, a hauler, and a
    // charger with zero amount, then asserts the hauler
    // routes to the charger and the charger's amount grows.
    let mut app = build_app();
    let deposit_pos = Vec2::new(100.0, 0.0);
    let cell = IVec2::new(2, 0);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 1000);
    let charger = common::spawn_charger_at(&mut app, cell, 0);
    let _hauler = common::spawn_hauler_at(&mut app, deposit_pos);
    // Paint a Defend cell so the charger auto-creation
    // system would not also create one (we have a manual
    // charger). The system only creates chargers in cells
    // with a holding defender; without a defender the
    // system does nothing.
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend, 1);

    // Drive enough ticks for the hauler to load (5 ticks at
    // HAULER_EXTRACT_PER_TICK) and walk from the deposit at
    // (100, 0) to the charger at the cell (2, 0) center
    // (~1280, 256). Distance ~ 1200 world units; at
    // bot_speed 5.0 = ~240 ticks. 500 is a safe margin.
    for _ in 0..500 {
        app.update();
    }

    let c = app.world().entity(charger).get::<Charger>().unwrap();
    let d = app
        .world()
        .entity(deposit)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
        .unwrap();
    assert!(
        c.amount > 0,
        "hauler must have delivered minerals to the charger; charger.amount = {}",
        c.amount
    );
    assert!(
        d.amount < 1000,
        "deposit must have lost material to the hauler; deposit.amount = {}",
        d.amount
    );
}

#[test]
fn defender_without_low_charge_does_not_rotate_to_charger() {
    // Companion: a defender with charge above
    // LOW_CHARGE_THRESHOLD must not be picked up by the
    // rotation system even if a working charger exists.
    // The defender stays in hold.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let _charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        // Above LOW_CHARGE_THRESHOLD: still strong, no rotation.
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = LOW_CHARGE_THRESHOLD + 0.1;
        w.entity_mut(defender).insert(DefendHold { cell });
    }

    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<ChargerAssignment>().is_none(),
        "fully-charged defender must not rotate to a charger"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_some(),
        "defender must stay in hold when charge is above the rotation threshold"
    );
}

#[test]
fn defender_charge_drains_passively_when_idle() {
    // Acceptance: "Defenders use Charge." A defender with
    // a Charge component must see the charge decrease over
    // time even when no charger is reachable. The drain
    // rate is CHARGE_DRAIN_PER_TICK per tick.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));
    let start = read_charge(&app, defender).expect("defender has Charge");
    assert_eq!(start, MAX_CHARGE);

    // Run 10 ticks without any Defend paint and without any
    // charger. The drain system must fire each tick.
    for _ in 0..10 {
        app.update();
    }
    let end = read_charge(&app, defender).expect("defender has Charge");
    // Pin the per-tick rate exactly: 10 ticks at
    // CHARGE_DRAIN_PER_TICK.
    let expected = MAX_CHARGE - CHARGE_DRAIN_PER_TICK * 10.0;
    assert!(
        (end - expected).abs() < 1e-5,
        "charge must drain by CHARGE_DRAIN_PER_TICK per tick; expected {expected}, got {end}"
    );
    // The drain must be strictly positive (the constant
    // matters, not just the sign).
    assert!(end < start, "charge must decrease over time");
}

#[test]
fn defender_charge_refills_faster_than_drain_at_a_working_charger() {
    // The "refill outpaces drain" contract: a defender at a
    // working charger must see the charge increase per tick
    // at CHARGE_REFILL_PER_TICK - CHARGE_DRAIN_PER_TICK.
    // The test plants the defender in ChargerProgress
    // state directly so arrival mechanics do not interfere
    // with the rate check.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.5;
        // Plant the defender in the charging state directly
        // so arrival mechanics are not in the way.
        w.entity_mut(defender).insert(ChargerAssignment { charger });
        w.entity_mut(defender).insert(ChargerProgress { charger });
    }
    let start = read_charge(&app, defender).expect("defender has Charge");
    assert!((start - 0.5).abs() < 1e-5);

    for _ in 0..5 {
        app.update();
    }
    let end = read_charge(&app, defender).expect("defender has Charge");
    // Per tick, the defender's charge goes up by REFILL and
    // down by DRAIN, for a net of (REFILL - DRAIN) per tick.
    // 5 ticks at the net rate. The defender may also have
    // left the charger (charge may have hit 1.0 and the
    // work system released them); the math below uses the
    // saturated end value.
    let net_per_tick = CHARGE_REFILL_PER_TICK - CHARGE_DRAIN_PER_TICK;
    let expected_saturated = (0.5 + net_per_tick * 5.0).min(MAX_CHARGE);
    assert!(
        (end - expected_saturated).abs() < 1e-4,
        "charge must refill at (REFILL - DRAIN) per tick; expected {expected_saturated}, got {end}"
    );
    assert!(
        end > start,
        "charge must increase while at a working charger"
    );
}

#[test]
fn defender_charger_assignment_does_not_block_defend_reassignment() {
    // Companion: a defender in the charging state must not
    // be re-routed to a fresh Defend cell by the defend
    // assignment system. The defend assignment system
    // filters out ChargerAssignment and ChargerProgress,
    // so a charging defender stays charging until the work
    // system releases them.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(0, 0);
    let other_cell = IVec2::new(2, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        other_cell,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );
    let cell_center = common::cell_world_center(cell);
    let charger = common::spawn_charger_at(&mut app, cell, 100);
    let defender = common::spawn_defender_at(&mut app, cell_center);
    {
        let w = app.world_mut();
        w.entity_mut(defender).get_mut::<Charge>().unwrap().current = 0.1;
        w.entity_mut(defender).insert(ChargerAssignment { charger });
        w.entity_mut(defender).insert(ChargerProgress { charger });
    }

    // The defend assignment system would normally pick up a
    // defender with no DefendHold and no DefendAssignment
    // and re-route them. The ChargerAssignment /
    // ChargerProgress filters must keep this defender out
    // of the routing pool.
    app.update();

    let world = app.world();
    assert!(
        world.entity(defender).get::<DefendAssignment>().is_none(),
        "defender with ChargerAssignment must not receive a DefendAssignment"
    );
    assert!(
        world.entity(defender).get::<DefendHold>().is_none(),
        "defender with ChargerAssignment must not enter DefendHold"
    );
}
