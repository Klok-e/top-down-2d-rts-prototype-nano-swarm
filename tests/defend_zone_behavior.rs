//! Integration tests for issue #13: Defend Zones and defender
//! combat behavior.
//!
//! Covers the four behaviours the issue calls out as acceptance
//! criteria:
//!   1. Defenders choose Defend Zone work from autonomy scoring.
//!   2. Defenders hold and protect painted areas.
//!   3. Defend Zone intent in enemy territory causes
//!      advance/attack behavior.
//!   4. Combat uses swarm systems rather than group commands
//!      (defenders are autonomous agents, each one is routed
//!      through the same scoring path as gatherers/builders).
//!
//! Tests are organised as a vertical-slice TDD loop. Each test
//! isolates one behavior so failures point at a single contract.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        best_candidate, is_enemy_territory, Commitment, DefendAssignment, DefendHold,
        DirectMovementComponent, NanobotType, SoftWorkSlots, DEFEND_HOME_RADIUS_CELLS,
    },
    ZONE_BLOCK_SIZE,
};

mod common;

fn build_app() -> App {
    common::sim_app_with_defend()
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
        assert!(grid.paint(cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
    }
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Sanity: the global scoring function picks this cell for an
    // idle Defender; this is the contract the assignment system
    // consumes.
    {
        let grid = app.world().resource::<IntentGrid>();
        let slots = app.world().resource::<SoftWorkSlots>();
        let picked = best_candidate(
            grid,
            NanobotType::Defender,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            slots,
            ZONE_BLOCK_SIZE,
            &[IntentKind::Defend],
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

    // The slot for the (cell, Defend) pair must be occupied so
    // future assignees see the cell as busier.
    let slots = app.world().resource::<SoftWorkSlots>();
    assert_eq!(
        slots.occupied(cell, IntentKind::Defend),
        1,
        "soft work slot must be occupied while the defender is assigned"
    );
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
        assert!(grid.paint(cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
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
fn defender_holds_position_at_defend_cell_after_arrival() {
    // Acceptance: "Defenders hold and protect painted areas."
    // A defender that has reached its assigned cell must enter
    // the hold state and stay at the cell center even when
    // separation forces push other bots nearby.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
    }
    let cell_center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // One update for the assignment; then enough updates to walk
    // from (0, 0) to (512, 256) at bot_speed 5.0 (about 110
    // ticks). 200 is a safe margin.
    for _ in 0..200 {
        app.update();
    }

    let world = app.world();
    let transform = world.entity(defender).get::<Transform>().unwrap();
    let distance_to_center = transform.translation.truncate().distance(cell_center);
    assert!(
        distance_to_center < 10.0,
        "defender should be at the cell center after travel; distance={distance_to_center}"
    );
    let hold = world
        .entity(defender)
        .get::<DefendHold>()
        .expect("defender should be in hold state after arrival");
    assert_eq!(hold.cell, cell);

    // The DefendAssignment has been promoted to DefendHold -- the
    // assignment is gone.
    assert!(
        world.entity(defender).get::<DefendAssignment>().is_none(),
        "DefendAssignment must be removed when the defender enters hold state"
    );

    // The slot is still occupied while the defender holds.
    let slots = world.resource::<SoftWorkSlots>();
    assert_eq!(
        slots.occupied(cell, IntentKind::Defend),
        1,
        "slot must remain occupied while the defender holds"
    );

    // The defender has no DirectMovementComponent while holding,
    // or a DMC that re-snaps to the cell center (the hold system
    // inserts a DMC if the defender drifted off-center; the move
    // system then immediately prunes it because distance <= STOP_THRESHOLD,
    // so the steady state is "no DMC").
    let dmc = world.entity(defender).get::<DirectMovementComponent>();
    match dmc {
        None => {}
        Some(d) => {
            assert!(
                (d.xy - cell_center).length() < 1.0,
                "defender DMC must be a re-snap to the cell center while holding; got {:?}",
                d.xy
            );
        }
    }
}

#[test]
fn defender_hold_releases_when_paint_erased() {
    // Hold contract: the hold persists "while the cell is still
    // painted". Erasing the Defend paint releases the slot and
    // drops the hold marker so the defender returns to the
    // assignment pool.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
    }
    let cell_center = common::cell_world_center(cell);
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Travel to the cell.
    for _ in 0..200 {
        app.update();
    }
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "precondition: defender must have arrived and entered hold"
    );

    // Erase the Defend paint.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.erase(cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
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
    // The defender position is still near the cell center; the
    // hold system did not move the defender, it just released
    // the marker.
    let transform = app.world().entity(defender).get::<Transform>().unwrap();
    let distance = transform.translation.truncate().distance(cell_center);
    assert!(
        distance < 10.0,
        "defender position unchanged by the hold release"
    );
}

#[test]
fn defender_advances_into_enemy_territory_when_paint_far_from_swarm() {
    // Acceptance: "Defend Zone intent in enemy territory causes
    // advance/attack behavior." A defender holding a friendly
    // Defend cell must re-route to a new Defend cell painted
    // further from the Swarm (i.e. in enemy territory) when the
    // player paints it. This is the "advance" half of the
    // hold/advance contract.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let swarm_cell = IVec2::new(0, 0);

    // Paint the first Defend cell at (1, 0) -- inside the home
    // radius, so friendly territory.
    let friendly_cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(friendly_cell, IntentKind::Defend, 8));
    }
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Travel to the friendly cell and enter hold.
    for _ in 0..200 {
        app.update();
    }
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_some(),
        "precondition: defender should be holding the friendly cell"
    );

    // Confirm the territory classification the test depends on:
    // the new cell is "enemy territory" because its Chebyshev
    // distance from the swarm is greater than the home radius.
    // The 8x8 grid spans [-4, 4), so (3, 0) is the farthest
    // in-bounds cell along the x-axis.
    let enemy_cell = IVec2::new(3, 0);
    assert!(
        is_enemy_territory(enemy_cell, swarm_cell, DEFEND_HOME_RADIUS_CELLS),
        "test fixture: enemy_cell must be in enemy territory relative to swarm_cell"
    );

    // Player paints a new Defend cell deep in enemy territory.
    // The defender should release hold and re-route.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(enemy_cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
    }
    app.update();
    assert!(
        app.world().entity(defender).get::<DefendHold>().is_none(),
        "defender must release hold when a new Defend cell becomes available further out"
    );
    let assignment = app
        .world()
        .entity(defender)
        .get::<DefendAssignment>()
        .expect("defender must be re-assigned to the new Defend cell");
    assert_eq!(
        assignment.cell, enemy_cell,
        "defender must be routed to the enemy-territory Defend cell (advance behavior)"
    );

    // The slot for the friendly cell is released on the way out
    // (the assignment system does not hold the old slot any
    // more); the enemy cell's slot is now occupied.
    let slots = app.world().resource::<SoftWorkSlots>();
    assert_eq!(
        slots.occupied(friendly_cell, IntentKind::Defend),
        0,
        "friendly cell slot must be released when the defender re-routes"
    );
    assert_eq!(
        slots.occupied(enemy_cell, IntentKind::Defend),
        1,
        "enemy cell slot must be occupied by the advancing defender"
    );

    // Travel to the enemy cell. Distance from (1, 0) center to
    // (3, 0) center is 2 * 512 = 1024 world units; at bot_speed
    // 5.0 that is 205 ticks. 300 is a safe margin.
    for _ in 0..300 {
        app.update();
    }

    let world = app.world();
    let enemy_cell_center = common::cell_world_center(enemy_cell);
    let transform = world.entity(defender).get::<Transform>().unwrap();
    let distance = transform.translation.truncate().distance(enemy_cell_center);
    assert!(
        distance < 10.0,
        "defender should have advanced to the enemy-territory cell; distance={distance}"
    );
    let hold = world
        .entity(defender)
        .get::<DefendHold>()
        .expect("defender should enter hold at the enemy-territory cell after the advance");
    assert_eq!(hold.cell, enemy_cell);
}

#[test]
fn multiple_defenders_route_independently_to_distinct_defend_cells() {
    // "Combat uses swarm systems rather than group commands" --
    // two defenders at the same starting point must end up at
    // distinct Defend cells, each routed independently through
    // the autonomy scorer. There is no "group move to cell A"
    // command; each defender picks its own best-scoring cell.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let left_cell = IVec2::new(-2, 0);
    let right_cell = IVec2::new(2, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(left_cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
        assert!(grid.paint(right_cell, IntentKind::Defend, PAINT_STRENGTH_CAP));
    }
    let d1 = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));
    let d2 = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // One update for the assignment system: each defender must
    // get a DefendAssignment. They can land on either cell; the
    // contract is that they land on different cells, because the
    // soft work slot pressure makes the second pick prefer the
    // empty cell.
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
