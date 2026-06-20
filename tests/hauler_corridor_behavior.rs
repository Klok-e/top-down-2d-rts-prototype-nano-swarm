//! Integration tests for issue #9: Logistics Corridors bias hauler paths.
//!
//! Each test isolates one behaviour so a failure points at a single
//! contract: corridor routing on the leg from idle to source, no
//! job creation without source/sink demand, paint-strength
//! preference, and the same routing on the leg from source to sink.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    game_settings::GameSettings,
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        bot_debug_circle_system, move_velocity_system, separation_system, velocity_system,
        Commitment, DirectMovementComponent, GatherPlugin, HaulPlugin, HaulerCorridorWaypoint,
        Nanobot, NanobotType, SoftWorkSlots, VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile},
    ZONE_BLOCK_SIZE,
};

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.insert_resource(IntentGrid::new(20, 20));
    app.insert_resource(GameSettings {
        width: 1000.0,
        height: 1000.0,
        bot_speed: 5.0,
        debug_draw_circles: false,
    });
    app.init_resource::<SoftWorkSlots>();
    app.init_resource::<ResourceLedger>();
    app.add_systems(
        Update,
        (
            separation_system,
            velocity_system,
            move_velocity_system,
            bot_debug_circle_system,
        )
            .chain(),
    );
    app.add_plugins(GatherPlugin);
    app.add_plugins(HaulPlugin);
    app
}

fn spawn_deposit(app: &mut App, world_pos: Vec2, amount: u32) -> Entity {
    app.world_mut()
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount,
                capacity: amount.max(1000),
                radius: 32.0,
            },
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

fn spawn_stockpile(app: &mut App, world_pos: Vec2, capacity: u32) -> Entity {
    app.world_mut()
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity,
                radius: 32.0,
            },
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

fn spawn_hauler_at(app: &mut App, world_pos: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            Commitment::Idle,
            VelocityComponent::default(),
            Transform::from_translation(world_pos.extend(0.0)),
        ))
        .id()
}

fn paint_corridor(app: &mut App, cell: IVec2, strength: u8) {
    assert!(app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Corridor,
        strength
    ));
}

fn corridor_cell_center(cell: IVec2) -> Vec2 {
    top_down_2d_rts_prototype_nano_swarm::ai::get_world_from_zone(cell)
}

#[test]
fn hauler_has_no_waypoint_without_corridor() {
    // Baseline: with no corridor painted, the hauler's
    // DirectMovementComponent must point straight at the
    // source. The corridor system must not invent a waypoint
    // for an empty corridor.
    let mut app = build_app();
    // Spawn the hauler far from the source so the DMC survives
    // a few ticks of movement instead of being removed on the
    // arrival check.
    let hauler_pos = Vec2::new(0.0, 0.0);
    let deposit_pos = Vec2::new(2_000.0, 0.0);
    let stockpile_pos = Vec2::new(3_000.0, 0.0);
    let _deposit = spawn_deposit(&mut app, deposit_pos, 1000);
    let _stockpile = spawn_stockpile(&mut app, stockpile_pos, 1000);
    let hauler = spawn_hauler_at(&mut app, hauler_pos);

    for _ in 0..3 {
        app.update();
    }

    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerCorridorWaypoint>()
            .is_none(),
        "hauler must not gain a corridor waypoint when no corridor is painted"
    );
    let dmc = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("hauler has a DMC after assignment");
    assert!(
        (dmc.xy - deposit_pos).length() < 1.0,
        "hauler DMC must point at the source when no corridor is painted; got {:?}",
        dmc.xy
    );
}

#[test]
fn hauler_picks_corridor_waypoint_when_painted_on_route() {
    // Painted corridor cell on the line from the hauler to the
    // source must become the hauler's waypoint. The DMC is
    // redirected through the corridor cell, so the hauler's
    // first leg bends toward the corridor instead of going
    // straight to the source.
    let mut app = build_app();
    let hauler_pos = Vec2::new(0.0, 0.0);
    let deposit_pos = Vec2::new(2_000.0, 0.0);
    let stockpile_pos = Vec2::new(3_000.0, 0.0);
    let _deposit = spawn_deposit(&mut app, deposit_pos, 1000);
    let _stockpile = spawn_stockpile(&mut app, stockpile_pos, 1000);
    let hauler = spawn_hauler_at(&mut app, hauler_pos);

    // Cell (2, 0) is on the line from (0, 0) to (2000, 0) and
    // sits between the hauler and the deposit. Painting it
    // gives a guaranteed-on-line waypoint.
    let painted = IVec2::new(2, 0);
    paint_corridor(&mut app, painted, PAINT_STRENGTH_CAP);

    for _ in 0..3 {
        app.update();
    }

    let waypoint = app
        .world()
        .entity(hauler)
        .get::<HaulerCorridorWaypoint>()
        .copied()
        .expect("hauler must gain a corridor waypoint when a corridor is painted on the route");
    let painted_center = corridor_cell_center(painted);
    assert!(
        (waypoint.waypoint - painted_center).length() < 1.0,
        "waypoint must be the painted cell's world center; got {:?}",
        waypoint.waypoint
    );
    assert!(
        (waypoint.target - deposit_pos).length() < 1.0,
        "waypoint target must be the source; got {:?}",
        waypoint.target
    );
    let dmc = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("hauler has a DMC after the corridor system fires");
    assert!(
        (dmc.xy - painted_center).length() < 1.0,
        "DMC must point at the corridor waypoint, not the source; got {:?}",
        dmc.xy
    );
}

#[test]
fn hauler_picks_higher_paint_corridor_cell() {
    // Two corridor cells on the same line, one with high paint
    // and one with low paint. The hauler system must prefer the
    // high-paint cell so Paint Strength can increase path
    // preference (acceptance criterion).
    let mut app = build_app();
    let hauler_pos = Vec2::new(0.0, 0.0);
    let deposit_pos = Vec2::new(2_000.0, 0.0);
    let stockpile_pos = Vec2::new(3_000.0, 0.0);
    let _deposit = spawn_deposit(&mut app, deposit_pos, 1000);
    let _stockpile = spawn_stockpile(&mut app, stockpile_pos, 1000);
    let hauler = spawn_hauler_at(&mut app, hauler_pos);

    let weak = IVec2::new(1, 0);
    let strong = IVec2::new(2, 0);
    paint_corridor(&mut app, weak, 1);
    paint_corridor(&mut app, strong, PAINT_STRENGTH_CAP);

    for _ in 0..3 {
        app.update();
    }

    let waypoint = app
        .world()
        .entity(hauler)
        .get::<HaulerCorridorWaypoint>()
        .copied()
        .expect("hauler must gain a waypoint with multiple painted cells");
    let strong_center = corridor_cell_center(strong);
    assert!(
        (waypoint.waypoint - strong_center).length() < 1.0,
        "hauler must prefer the high-paint cell; got waypoint {:?}, expected {:?}",
        waypoint.waypoint,
        strong_center
    );
}

#[test]
fn corridor_only_intent_does_not_create_hauling_job() {
    // The acceptance criterion is explicit: a corridor cell
    // alone must not produce a HaulerAssignment. A hauler with
    // no source and no sink nearby stays idle even when a
    // corridor is painted at its feet.
    let mut app = build_app();
    let hauler = spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));
    paint_corridor(&mut app, IVec2::new(0, 0), PAINT_STRENGTH_CAP);

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(hauler)
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::HaulerAssignment>()
            .is_none(),
        "corridor must not create a HaulerAssignment"
    );
    assert!(
        app.world()
            .entity(hauler)
            .get::<DirectMovementComponent>()
            .is_none(),
        "corridor must not give the hauler a destination"
    );
    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerCorridorWaypoint>()
            .is_none(),
        "corridor must not give the hauler a waypoint without an active trip"
    );
}

#[test]
fn hauler_routes_through_corridor_to_sink_after_loading() {
    // Acceptance bullet: "Haulers prefer corridor-influenced
    // paths when transporting resources." The carry leg (from
    // source to sink) is the visible part of the trip. Painting
    // a corridor between the source and the sink must make the
    // hauler's DMC point at the corridor cell on the carry
    // leg, not straight at the sink.
    let mut app = build_app();
    // Two cells apart on the x-axis keeps the deposit and
    // stockpile easy to identify while staying on a single
    // line of cells for the corridor.
    let deposit_pos = Vec2::new(0.0, 0.0);
    let stockpile_pos = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
    let deposit = spawn_deposit(&mut app, deposit_pos, 1000);
    let stockpile = spawn_stockpile(&mut app, stockpile_pos, 1000);
    let hauler = spawn_hauler_at(&mut app, deposit_pos);

    // Paint a corridor cell on the line between source and sink.
    // Cell (1, 0) is between (0, 0) and (3, 0) on the x-axis.
    let painted = IVec2::new(1, 0);
    paint_corridor(&mut app, painted, PAINT_STRENGTH_CAP);

    // Pre-seed the assignment so the test isolates the corridor
    // effect from the assignment selection.
    app.world_mut().entity_mut(hauler).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::HaulerAssignment {
            source: deposit,
            sink: stockpile,
        },
    );

    // Drive ticks until the corridor waypoint first appears on
    // the carry leg, then capture the waypoint + DMC snapshot.
    // 5 load ticks (HAULER_EXTRACT_PER_TICK into
    // HAULER_CARRY_CAPACITY) + 1 carry-assign tick is enough.
    let mut waypoint_at_appearance: Option<HaulerCorridorWaypoint> = None;
    let mut dmc_xy_at_appearance: Option<Vec2> = None;
    for _ in 0..10 {
        app.update();
        if waypoint_at_appearance.is_none()
            && app
                .world()
                .entity(hauler)
                .get::<HaulerCorridorWaypoint>()
                .is_some()
        {
            waypoint_at_appearance = app
                .world()
                .entity(hauler)
                .get::<HaulerCorridorWaypoint>()
                .copied();
            dmc_xy_at_appearance = app
                .world()
                .entity(hauler)
                .get::<DirectMovementComponent>()
                .map(|dmc| dmc.xy);
        }
    }

    let waypoint = waypoint_at_appearance
        .expect("hauler must gain a corridor waypoint on the carry leg when a corridor is painted between source and sink");
    let dmc_xy =
        dmc_xy_at_appearance.expect("hauler must keep a DMC while the corridor waypoint is active");

    let painted_center = corridor_cell_center(painted);
    assert!(
        (waypoint.waypoint - painted_center).length() < 1.0,
        "waypoint must be the painted cell's world center; got {:?}",
        waypoint.waypoint
    );
    assert!(
        (waypoint.target - stockpile_pos).length() < 1.0,
        "waypoint target must be the sink; got {:?}",
        waypoint.target
    );
    assert!(
        (dmc_xy - painted_center).length() < 1.0,
        "DMC must point at the corridor waypoint, not the sink; got {:?}",
        dmc_xy
    );
}
