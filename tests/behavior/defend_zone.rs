//! Integration tests for the regional Defender allocation and
//! in-cell holding contract.
//!
//! Tests isolate assignment, lease stability, arrival, containment,
//! paint invalidation, and movement-feel regressions.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        AllocationRegion, DEFEND_IN_CELL_STOP_RADIUS, DefendAssignment, DefendHold, DefendPressure,
        DirectMovementComponent, OpportunityCategory, OpportunityTarget, RegionalLease, SwarmId,
        VelocityComponent, point_in_cell,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_defend()
}

#[test]
fn combined_direct_and_separation_velocity_is_clamped_to_bot_speed() {
    let mut app = common::sim_app();
    let mover = common::spawn_defender_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::X);
    app.world_mut()
        .entity_mut(mover)
        .insert(DirectMovementComponent {
            xy: Vec2::new(-100.0, 0.0),
            stop_radius: 0.0,
        });

    app.update();

    let position = app
        .world()
        .entity(mover)
        .get::<Transform>()
        .expect("mover transform")
        .translation
        .truncate();
    assert!(
        position.x < -4.9,
        "direct movement and separation should push left"
    );
    assert!(
        position.length() <= common::default_game_settings().bot_speed + 1e-4,
        "combined velocity must not exceed bot speed; displacement={position}"
    );
}

#[test]
fn coincident_crowd_velocity_remains_finite() {
    let mut app = common::sim_app();
    let crowd = (0..8)
        .map(|_| common::spawn_worker_at(&mut app, Vec2::ZERO))
        .collect::<Vec<_>>();

    app.update();

    for entity in crowd {
        let world = app.world();
        let transform = world
            .entity(entity)
            .get::<Transform>()
            .expect("crowd transform");
        let velocity = world
            .entity(entity)
            .get::<VelocityComponent>()
            .expect("crowd velocity");
        assert!(transform.translation.is_finite());
        assert!(velocity.value.is_finite());
    }
}

#[test]
fn crowded_holder_remains_inside_supported_cell() {
    let mut app = build_app();
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Defend);
    let defenders = (0..8)
        .map(|_| {
            let entity = common::spawn_defender_at(&mut app, center);
            app.world_mut()
                .entity_mut(entity)
                .insert(DefendHold { cell });
            entity
        })
        .collect::<Vec<_>>();

    for _ in 0..240 {
        app.update();
    }

    for entity in defenders {
        let position = app
            .world()
            .entity(entity)
            .get::<Transform>()
            .expect("holder transform")
            .translation
            .truncate();
        assert!(
            point_in_cell(position, cell),
            "crowded holder escaped supported cell: {position}"
        );
    }
}

/// Spawn a defender, plant it at `cell`'s world center in a
/// `DefendHold`, and attach an active regional lease.
/// Used by lease and paint-invalidation tests to start from a known
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
fn idle_defender_receives_defend_assignment_from_regional_allocator() {
    // Regional allocation is the single source of Defender work.
    // A supported painted cell must produce a movement assignment.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let cell = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(cell, IntentKind::Defend));
    }
    let defender = common::spawn_defender_at(&mut app, Vec2::new(0.0, 0.0));

    // Drive the regional allocator; the defender should receive a
    // DefendAssignment pointing at the cell.
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
    // Regional allocation only offers Defend work to Defenders.
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
    // painted". Erasing the Defend paint releases the regional
    // lease and drops the hold marker so the defender returns to the
    // assignment pool. The defender's position is unchanged by
    // the release -- only the marker and lease move.
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
    assert!(
        app.world()
            .entity(defender)
            .get::<RegionalLease>()
            .is_none(),
        "regional lease must be released when the paint is erased"
    );
    // The defender position is unchanged by the hold release.
    let pos_after = app
        .world()
        .entity(defender)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    assert!(
        pos_before.distance(pos_after) <= 0.01,
        "defender position unchanged by the hold release"
    );
}

#[test]
fn multiple_defenders_route_independently_to_distinct_defend_cells() {
    // "Combat uses swarm systems rather than group commands" --
    // two defenders at the same starting point must end up at
    // distinct Defend cells through deterministic regional claims.
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

    // One update for the regional allocator: each defender must
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
fn active_holder_keeps_lease_when_other_cell_pressure_rises() {
    // A pressure update elsewhere must not displace an active holder.
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
        "regional allocation must not displace an active holder"
    );
}

#[test]
fn holding_defender_retargets_immediately_when_current_paint_erased() {
    // Acceptance: "erased current paint releases or retargets them
    // immediately." When the held cell's paint is erased and a new
    // Defend cell is painted elsewhere (the player moving intent
    // forward -- the "advance" path), the defender releases and
    // retargets on the next allocation pass.
    let mut app = build_app();
    let _swarm = common::spawn_swarm_at(&mut app, Vec2::new(0.0, 0.0));
    let friendly = IVec2::new(1, 0);
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint(friendly, IntentKind::Defend));
    }
    let defender = spawn_holding_defender(&mut app, friendly);

    // Paint a new front cell after withdrawing the rear cell.
    let enemy = IVec2::new(3, 0);

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
        .expect("defender must retarget to the newly painted cell");
    assert_eq!(assignment.cell, enemy);

    // Travel to the enemy cell and enter hold there.
    for _ in 0..300 {
        app.update();
    }
    let world = app.world();
    let transform = world.entity(defender).get::<Transform>().unwrap();
    assert!(
        point_in_cell(transform.translation.truncate(), enemy),
        "defender should have advanced into the newly painted cell"
    );
    let hold = world
        .entity(defender)
        .get::<DefendHold>()
        .expect("defender should hold the newly painted cell after advancing");
    assert_eq!(hold.cell, enemy);
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
    app.world_mut()
        .entity_mut(defender)
        .insert(DefendHold { cell: held });

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

// All imports above are used by at least one test.
