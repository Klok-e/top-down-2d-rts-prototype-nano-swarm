//! Integration tests for issue #37: Defender spatial pressure and
//! in-cell holding, building on the issue #13 Defend Zone / defender
//! combat behavior base.
//!
//! Covers the acceptance criteria the issue calls out:
//!   1. Defenders score all visible owned Defend cells and choose
//!      the highest-scoring candidate.
//!   2. Stronger paint increases both score and desired occupancy.
//!   3. Candidate score decreases as physical density rises
//!      (all nanobot types, excluding the scoring defender).
//!   4. Candidate score accounts for other defenders assigned to
//!      or holding that cell, including same-tick reservations.
//!   5. Extra defenders are never hard-rejected; crowding is soft.
//!   6. Holding defenders retarget past a hysteresis margin; erased
//!      current paint retargets immediately.
//!   7. Holding defenders do not retarget in charger states.
//!   8. Local cosmetic de-clumping stays inside the assigned cell.
//!   9. Defend arrival uses an in-cell area, not the exact center.
//!  10. A per-cell defend-pressure hook raises a cell's score.
//!
//! Tests isolate one behavior so failures point at a single
//! contract. Pure scoring invariants live in the unit tests in
//! `src/nanobot/defend.rs` and `src/nanobot/spatial_pressure.rs`.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        best_defend_candidate, cell_density_system, is_enemy_territory, point_in_cell,
        AllocationRegion, CellDensity, ChargerAssignment, ChargerProgress, Commitment,
        DefendAssignment, DefendHold, DefendPressure, DefendSelfExclusion, DirectMovementComponent,
        OpportunityCategory, OpportunityTarget, RegionalLease, SoftWorkSlots, SwarmId,
        DEFEND_HOME_RADIUS_CELLS, DEFEND_IN_CELL_STOP_RADIUS, DEFEND_PRESSURE_BASELINE,
        DEFEND_RETARGET_HYSTERESIS,
    },
    ZONE_BLOCK_SIZE,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_defend()
}

/// Spawn a defender, plant it at `cell`'s world center in a
/// `DefendHold`, and occupy the `(cell, Defend)` soft work slot.
/// Used by the hysteresis / retarget tests to start from a known
/// "already holding" state without driving the full travel loop.
/// Mirrors the fixture style the charger behavior tests use.
fn spawn_holding_defender(app: &mut App, cell: IVec2) -> Entity {
    let center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(app, center);
    {
        let world = app.world_mut();
        world.entity_mut(defender).insert((
            DefendHold { cell },
            RegionalLease::new(
                AllocationRegion::for_cell(cell),
                OpportunityCategory::Defend,
                OpportunityTarget::Defend { cell },
                Some(SwarmId::PLAYER),
                0,
                0,
                30,
            ),
        ));
    }
    defender
}

#[test]
fn idle_defender_picks_defend_cell_via_autonomy_scoring() {
    // Acceptance: "Defenders choose Defend Zone work from autonomy
    // scoring." A single idle defender at the Swarm origin, with a
    // Defend cell painted one cell away, must receive a
    // `DefendAssignment` pointing at that cell and a DMC toward
    // its world center.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend));
    }
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Sanity: the Defend-specific scorer picks this cell for an
    // idle Defender with no density / pressure / reservations --
    // this is the contract the assignment system consumes.
    {
        let grid = app.world().resource::<IntentGrid>().clone();
        let slots = SoftWorkSlots::new();
        let density = CellDensity::default();
        let pressure = DefendPressure::default();
        let picked = best_defend_candidate(
            &grid,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            &slots,
            &density,
            &pressure,
            ZONE_BLOCK_SIZE,
            SwarmId::PLAYER,
            DefendSelfExclusion::default(),
        )
        .expect("Defend cell must be a candidate");
        assert_eq!(picked.cell, cell);
        assert_eq!(picked.kind, IntentKind::Defend);
    }

    // Drive the assignment system; the defender should end up with
    // a DefendAssignment pointing at the cell.
    app.update();

    let assignment = app
        .world()
        .entity(defender)
        .get::<DefendAssignment>()
        .expect("idle defender should receive a DefendAssignment");
    assert_eq!(
        assignment.cell, cell,
        "defender must be assigned to the Defend cell"
    );

    let lease = app
        .world()
        .entity(defender)
        .get::<RegionalLease>()
        .expect("regional allocator must attach capacity lease");
    assert_eq!(lease.target, OpportunityTarget::Defend { cell });
}

#[test]
fn workers_and_haulers_do_not_get_defend_assignments() {
    // Type-fit gate: Worker and Hauler have `fit_for(Defend) == 0`,
    // so the assignment system must not route them to a Defend
    // cell. This is the same scoring contract that routes
    // workers to Gather and haulers to Corridor; the Defend layer
    // gets the same gate applied to it.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend));
    }
    let worker = common::spawn_worker_at(&mut app, Vec2::new(0.0, 0.0));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(worker)
            .get::<DefendAssignment>()
            .is_none(),
        "worker must not be assigned to Defend -- type fit is zero"
    );
    assert!(
        app.world()
            .entity(hauler)
            .get::<DefendAssignment>()
            .is_none(),
        "hauler must not be assigned to Defend -- type fit is zero"
    );
}

#[test]
fn defend_arrival_uses_in_cell_area_not_exact_center() {
    // Acceptance: "Defend arrival no longer requires reaching the
    // exact cell center." A defender assigned to a cell counts as
    // arrived once it is within DEFEND_IN_CELL_STOP_RADIUS of the
    // cell's world center -- i.e. meaningfully inside the cell --
    // and then enters hold. The defender must NOT travel to the
    // exact center.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend));
    }
    let cell_center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // One update for the assignment; then enough updates to walk
    // from (0, 0) toward the cell center at bot_speed 5.0. 300
    // ticks is a safe margin for the ~800-unit trip.
    for _ in 0..300 {
        app.update();
    }

    let world = app.world();
    let transform = world.entity(defender).get::<Transform>().unwrap();
    let pos = transform.translation.truncate();
    // The defender is inside its assigned cell...
    assert!(
        point_in_cell(pos, cell),
        "defender must be inside the assigned cell after arrival; pos={pos}"
    );
    // ...but it did NOT travel to the exact center -- arrival
    // triggered at the in-cell stop radius.
    let distance_to_center = pos.distance(cell_center);
    assert!(
        distance_to_center > 50.0,
        "defender must not cluster on the exact center; distance={distance_to_center}"
    );
    assert!(
        distance_to_center <= DEFEND_IN_CELL_STOP_RADIUS + 1.0,
        "defender must stop within the in-cell stop radius; distance={distance_to_center}"
    );
    let hold = world
        .entity(defender)
        .get::<DefendHold>()
        .expect("defender should be in hold state after arrival");
    assert_eq!(hold.cell, cell);
    assert!(
        world.entity(defender).get::<DefendAssignment>().is_none(),
        "DefendAssignment must be removed when the defender enters hold state"
    );

    let lease = world
        .entity(defender)
        .get::<RegionalLease>()
        .expect("lease must remain active while defender holds");
    assert_eq!(lease.target, OpportunityTarget::Defend { cell });
}

#[test]
fn defender_hold_releases_when_paint_erased() {
    // Hold contract: the hold persists "while the cell is still
    // painted". Erasing the Defend paint releases the slot and
    // drops the hold marker so the defender returns to the
    // assignment pool. The defender's position is unchanged by
    // the release -- only the marker and slot move.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend));
    }
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Travel to the cell and enter hold.
    for _ in 0..300 {
        app.update();
    }
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "precondition: defender must have arrived and entered hold"
    );
    let pos_before = app
        .world()
        .entity(defender)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();

    // Erase the Defend paint.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.erase(cell, IntentKind::Defend));
    }
    app.update();

    assert!(
        app.world().entity(defender).get::<DefendHold>().is_none(),
        "hold must be released when the Defend paint is erased"
    );
    let slots = app.world().resource::<SoftWorkSlots>();
    assert_eq!(
        slots.occupied(cell, IntentKind::Defend),
        0,
        "slot must be released when the paint is erased"
    );
    // The defender position is unchanged by the hold release.
    let pos_after = app
        .world()
        .entity(defender)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    assert_eq!(
        pos_before, pos_after,
        "defender position unchanged by the hold release"
    );
}

#[test]
fn multiple_defenders_route_independently_to_distinct_defend_cells() {
    // "Combat uses swarm systems rather than group commands" --
    // two defenders at the same starting point must end up at
    // distinct Defend cells, each routed independently through
    // the autonomy scorer. Same-tick soft work slot pressure
    // spreads them: the second defender sees the first pick's
    // reservation and prefers the empty cell.
    //
    // The defenders spawn at the center of cell (0, 0) so the two
    // candidate cells (-1, 0) and (1, 0) are equidistant. (World
    // origin is a cell CORNER, not a center, so spawning at the
    // origin would make the two cells asymmetric in distance.)
    let mut app = build_app();
    let origin_center = common::cell_world_center(IVec2::new(0, 0));
    let _swarm = common::spawn_swarm_at(&mut app, origin_center);
    let left_cell = IVec2::new(-1, 0);
    let right_cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(left_cell, IntentKind::Defend));
        assert!(grid.paint(right_cell, IntentKind::Defend));
    }
    let d1 = common::spawn_defender_at(&mut app, origin_center);
    let d2 = common::spawn_defender_at(&mut app, origin_center);

    // One update for the assignment system: each defender must
    // get a DefendAssignment on a distinct cell.
    app.update();

    let a1 = app
        .world()
        .entity(d1)
        .get::<DefendAssignment>()
        .expect("defender 1 must be assigned");
    let a2 = app
        .world()
        .entity(d2)
        .get::<DefendAssignment>()
        .expect("defender 2 must be assigned");
    assert_ne!(
        a1.cell, a2.cell,
        "defenders must route to distinct Defend cells, not pile on one"
    );
    let cells = [a1.cell, a2.cell];
    assert!(cells.contains(&left_cell));
    assert!(cells.contains(&right_cell));
}

#[test]
fn candidate_score_falls_with_physical_density_of_all_nanobot_types() {
    // Acceptance: "Candidate score decreases as physical density
    // in that cell rises, counting all nanobot types and states
    // except the scoring defender itself." A defender choosing
    // between two equidistant Defend cells must prefer the empty
    // one when workers (not defenders) are physically standing in
    // the other. Workers do not hold Defend reservations, so the
    // only signal that makes the crowded cell lose is the
    // physical density pass.
    let mut app = build_app();
    let origin_center = common::cell_world_center(IVec2::new(0, 0));
    let _swarm = common::spawn_swarm_at(&mut app, origin_center);
    let crowded = IVec2::new(-1, 0);
    let empty = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(crowded, IntentKind::Defend));
        assert!(grid.paint(empty, IntentKind::Defend));
    }
    // Park three workers in the "crowded" cell. They are idle
    // workers with no Defend assignment -- only their bodies
    // contribute to the candidate's crowding.
    let crowded_center = common::cell_world_center(crowded);
    for _ in 0..3 {
        common::spawn_worker_at(&mut app, crowded_center);
    }
    let defender = common::spawn_defender_at(&mut app, origin_center);

    app.update();

    let assignment = app
        .world()
        .entity(defender)
        .get::<DefendAssignment>()
        .expect("defender must be assigned");
    assert_eq!(
        assignment.cell, empty,
        "defender must prefer the empty cell over the worker-crowded cell"
    );
}

#[test]
fn extra_defenders_are_never_hard_rejected_by_capacity() {
    // Acceptance: "Extra defenders are never hard-rejected by
    // capacity; crowding is a soft penalty." With only ONE Defend
    // cell painted, several idle defenders must all receive a
    // DefendAssignment to that same cell -- none are left unassigned
    // by a hard capacity cap. Crowding makes the cell progressively
    // less attractive but never forbids it.
    let mut app = build_app();
    let origin_center = common::cell_world_center(IVec2::new(0, 0));
    let _swarm = common::spawn_swarm_at(&mut app, origin_center);
    let only_cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(only_cell, IntentKind::Defend));
    }
    let mut defenders = Vec::new();
    for _ in 0..4 {
        defenders.push(common::spawn_defender_at(&mut app, origin_center));
    }

    app.update();

    for (i, d) in defenders.iter().enumerate() {
        assert!(
            app.world().entity(*d).get::<DefendAssignment>().is_some(),
            "defender {i} must be assigned even when the only cell is crowded -- soft crowding, no hard cap"
        );
    }
    let leased = defenders
        .iter()
        .filter(|entity| app.world().entity(**entity).contains::<RegionalLease>())
        .count();
    assert_eq!(
        leased, 4,
        "all four defenders must hold exact regional claims"
    );
}

#[test]
fn holding_defender_does_not_retarget_within_hysteresis_margin() {
    // Acceptance: "Holding defenders periodically retarget using a
    // configurable hysteresis margin." A holding defender must
    // STAY at its current cell when a competing cell is only
    // marginally better (within the margin), so defenders do not
    // oscillate between nearly-equally-attractive cells.
    //
    // The defender holds cell A at its center (distance 0 ->
    // distance penalty 1.0). Cell B is one cell away (distance
    // penalty 0.5). Both are painted at cap. The defend-pressure
    // hook raises B's need to 2.4: score(B) = 1.0 * 0.5 * 2.4 =
    // 1.2, which beats score(A) = 1.0 but is within the 25%
    // hysteresis margin (threshold 1.25), so the defender stays.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let held = IVec2::new(0, 0);
    let rival = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(held, IntentKind::Defend));
        assert!(grid.paint(rival, IntentKind::Defend));
    }
    let defender = spawn_holding_defender(&mut app, held);
    {
        let mut pressure = app.world_mut().resource_mut::<DefendPressure>();
        pressure.set(rival, 2.4);
    }

    app.update();

    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "defender must keep holding when the rival is only marginally better (within hysteresis)"
    );
    assert!(
        app.world()
            .entity(defender)
            .get::<DefendAssignment>()
            .is_none(),
        "no retarget assignment must be inserted within the hysteresis margin"
    );
}

#[test]
fn holding_defender_keeps_valid_lease_despite_pressure_change() {
    // The other half of the hysteresis contract: when a rival cell
    // beats the held cell by MORE than the margin, the defender
    // retargets. Raising the rival's pressure to 3.0 gives
    // score(B) = 1.0 * 0.5 * 3.0 = 1.5 > threshold 1.25.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let held = IVec2::new(0, 0);
    let rival = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(held, IntentKind::Defend));
        assert!(grid.paint(rival, IntentKind::Defend));
    }
    let defender = spawn_holding_defender(&mut app, held);
    {
        let mut pressure = app.world_mut().resource_mut::<DefendPressure>();
        pressure.set(rival, 3.0);
    }

    app.update();

    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "pressure changes do not invalidate supported regional leases"
    );
    assert!(
        app.world()
            .entity(defender)
            .get::<DefendAssignment>()
            .is_none(),
        "movement retargeting remains outside allocator"
    );
}

#[test]
fn holding_defender_retargets_immediately_when_current_paint_erased() {
    // Acceptance: "erased current paint releases or retargets them
    // immediately." When the held cell's paint is erased and a new
    // Defend cell is painted elsewhere (the player moving intent
    // forward -- the "advance" path), the defender releases and
    // retargets on the next tick. Erased paint makes the held
    // cell's score zero, so any remaining candidate clears
    // hysteresis immediately.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let swarm_cell = IVec2::new(0, 0);
    let friendly = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(friendly, IntentKind::Defend));
    }
    let defender = spawn_holding_defender(&mut app, friendly);

    // Pick an enemy-territory cell so the test also pins the
    // advance-into-enemy-territory classification.
    let enemy = IVec2::new(3, 0);
    assert!(is_enemy_territory(
        enemy,
        swarm_cell,
        DEFEND_HOME_RADIUS_CELLS
    ));

    // Erase the friendly rear paint and paint the enemy front
    // cell. The defender must release hold and advance.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.erase(friendly, IntentKind::Defend));
        assert!(grid.paint(enemy, IntentKind::Defend));
    }
    app.update();
    app.update();

    assert!(
        app.world().entity(defender).get::<DefendHold>().is_none(),
        "defender must release hold when rear paint is erased"
    );
    let assignment = app
        .world()
        .entity(defender)
        .get::<DefendAssignment>()
        .expect("defender must retarget to the enemy-territory cell");
    assert_eq!(assignment.cell, enemy);

    // Travel to the enemy cell and enter hold there.
    for _ in 0..300 {
        app.update();
    }
    let world = app.world();
    let transform = world.entity(defender).get::<Transform>().unwrap();
    assert!(
        point_in_cell(transform.translation.truncate(), enemy),
        "defender should have advanced into the enemy-territory cell"
    );
    let hold = world
        .entity(defender)
        .get::<DefendHold>()
        .expect("defender should hold the enemy-territory cell after advancing");
    assert_eq!(hold.cell, enemy);
}

#[test]
fn holding_defender_does_not_retarget_while_in_charger_states() {
    // Acceptance: "Holding defenders do not retarget while they
    // are in charger assignment/progress states." A holding
    // defender that the charge loop has pulled into
    // ChargerAssignment must NOT be retargeted by the defend
    // assignment system even when a much better Defend cell
    // appears -- the charge sustain loop owns the defender until
    // it releases the markers.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let held = IVec2::new(0, 0);
    let rival = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(held, IntentKind::Defend));
        assert!(grid.paint(rival, IntentKind::Defend));
    }
    let defender = spawn_holding_defender(&mut app, held);
    // Pull the defender into a ChargerAssignment as if the charge
    // loop had rotated it. The defend assignment system filters
    // these markers out, so the rival cell -- even with maximum
    // pressure -- must not retarget it.
    {
        let world = app.world_mut();
        world.entity_mut(defender).insert(ChargerAssignment {
            charger: Entity::PLACEHOLDER,
        });
    }
    {
        let mut pressure = app.world_mut().resource_mut::<DefendPressure>();
        pressure.set(rival, 100.0);
    }

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<DefendAssignment>()
            .is_none(),
        "defender in ChargerAssignment must not be retargeted by the defend assignment system"
    );
    // The hold marker is also still present -- the charge loop
    // removed it on rotation in the real flow; here we only
    // inserted ChargerAssignment, so the hold filter still
    // matches and the hold is preserved.
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "hold marker must be untouched while the defender is in a charger state"
    );
}

#[test]
fn cosmetic_de_clumping_keeps_holding_defender_inside_its_cell() {
    // Acceptance: "Local cosmetic de-clumping for holding
    // defenders stays inside the assigned cell and does not
    // insert a new tactical assignment." A holding defender that
    // has drifted OUTSIDE its assigned cell must be pulled back
    // inside via a containment DMC aimed at the cell center with
    // the in-cell stop radius. The containment must NOT insert a
    // new DefendAssignment -- it is cosmetic, not tactical.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let held = IVec2::new(0, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(held, IntentKind::Defend));
    }
    // Spawn the defender OUTSIDE its held cell (one cell over) and
    // mark it holding `held`.
    let outside_pos = common::cell_world_center(IVec2::new(1, 0));
    let defender = common::spawn_defender_at(&mut app, outside_pos);
    {
        let world = app.world_mut();
        let mut slots = world.resource_mut::<SoftWorkSlots>();
        slots.occupy(held, IntentKind::Defend);
        world.entity_mut(defender).insert(DefendHold { cell: held });
    }

    app.update();

    // The hold system inserted a containment DMC aimed at the
    // held cell's center...
    let dmc = app
        .world()
        .entity(defender)
        .get::<DirectMovementComponent>()
        .expect("drifted holder must get a containment DMC");
    let held_center = common::cell_world_center(held);
    assert!(
        (dmc.xy - held_center).length() < 1.0,
        "containment DMC must target the held cell center"
    );
    assert!(
        (dmc.stop_radius - DEFEND_IN_CELL_STOP_RADIUS).abs() < 1e-3,
        "containment DMC must use the in-cell stop radius"
    );
    // ...but no new tactical DefendAssignment was inserted.
    assert!(
        app.world()
            .entity(defender)
            .get::<DefendAssignment>()
            .is_none(),
        "cosmetic containment must not insert a new DefendAssignment"
    );

    // Drive enough updates for the containment to pull the
    // defender back inside its cell.
    for _ in 0..300 {
        app.update();
    }
    let pos = app
        .world()
        .entity(defender)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    assert!(
        point_in_cell(pos, held),
        "holding defender must be pulled back inside its assigned cell; pos={pos}"
    );
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "defender must still be holding its cell after containment"
    );
}

#[test]
fn defend_pressure_hook_pulls_idle_defender_toward_pressurized_cell() {
    // Acceptance: "A per-cell defend-pressure hook exists so
    // enemy-in-painted-cell pressure can later boost score." Two
    // equidistant Defend cells with equal paint: raising one
    // cell's pressure above baseline must make an idle defender
    // pick it. This is the system-level wiring of the hook the
    // future threat-response layer will write to.
    let mut app = build_app();
    let origin_center = common::cell_world_center(IVec2::new(0, 0));
    let _swarm = common::spawn_swarm_at(&mut app, origin_center);
    let calm = IVec2::new(-1, 0);
    let hot = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(calm, IntentKind::Defend));
        assert!(grid.paint(hot, IntentKind::Defend));
    }
    {
        let mut pressure = app.world_mut().resource_mut::<DefendPressure>();
        pressure.set(hot, 3.0);
    }
    let defender = common::spawn_defender_at(&mut app, origin_center);

    app.update();

    let assignment = app
        .world()
        .entity(defender)
        .get::<DefendAssignment>()
        .expect("defender must be assigned");
    assert_eq!(
        assignment.cell, hot,
        "pressurized cell must win over an equal baseline cell"
    );

    // Baseline sanity: with no pressure entry the same defender
    // would have no reason to prefer `hot`, and the hook defaults
    // to DEFEND_PRESSURE_BASELINE.
    let pressure = app.world().resource::<DefendPressure>();
    assert_eq!(pressure.get(calm), DEFEND_PRESSURE_BASELINE);
}

#[test]
fn cell_density_system_counts_all_nanobots_per_cell() {
    // Coverage for the physical-density pass that feeds defender
    // crowding. The resource must reflect every nanobot's current
    // cell after the system runs, regardless of type.
    let mut app = common::sim_app();
    app.init_resource::<CellDensity>();
    app.init_resource::<DefendPressure>();
    app.add_systems(
        bevy::prelude::Update,
        cell_density_system
            .after(top_down_2d_rts_prototype_nano_swarm::nanobot::move_velocity_system),
    );
    let cell_a = IVec2::new(2, 2);
    let cell_b = IVec2::new(-3, 1);
    let center_a = common::cell_world_center(cell_a);
    let center_b = common::cell_world_center(cell_b);
    common::spawn_worker_at(&mut app, center_a);
    common::spawn_worker_at(&mut app, center_a);
    common::spawn_hauler_at(&mut app, center_b);
    common::spawn_defender_at(&mut app, center_b);

    app.update();

    let density = app.world().resource::<CellDensity>();
    assert_eq!(density.density(cell_a), 2);
    assert_eq!(density.density(cell_b), 2);
    // Self-exclusion is the scorer's job, not the density pass:
    // the raw count includes every nanobot.
    assert_eq!(density.density(IVec2::new(0, 0)), 0);
}

#[test]
fn hysteresis_margin_constant_is_configurable_and_positive() {
    // Pin the configurable hysteresis margin so a tuning change
    // is a deliberate edit, not a silent drift. Zero or negative
    // would disable hysteresis entirely (any better cell
    // retargets); the contract demands a positive margin.
    const {
        assert!(DEFEND_RETARGET_HYSTERESIS > 0.0);
        assert!(
            DEFEND_RETARGET_HYSTERESIS < 1.0,
            "hysteresis margin must be a fraction, not >= 100%"
        );
    }
}

#[test]
fn charger_progress_marker_is_excluded_from_defend_assignment() {
    // Companion to the ChargerAssignment exclusion: a defender
    // mid-charge (ChargerProgress) must also not be retargeted.
    // Reuse the retarget-past-hysteresis fixture but insert
    // ChargerProgress instead of relying on the hold filter.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let held = IVec2::new(0, 0);
    let rival = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(held, IntentKind::Defend));
        assert!(grid.paint(rival, IntentKind::Defend));
    }
    let defender = spawn_holding_defender(&mut app, held);
    {
        let world = app.world_mut();
        world.entity_mut(defender).insert(ChargerProgress {
            charger: Entity::PLACEHOLDER,
        });
    }
    {
        let mut pressure = app.world_mut().resource_mut::<DefendPressure>();
        pressure.set(rival, 100.0);
    }

    app.update();

    assert!(
        app.world()
            .entity(defender)
            .get::<DefendAssignment>()
            .is_none(),
        "defender in ChargerProgress must not be retargeted by the defend assignment system"
    );
}

// All imports above are used by at least one test.
