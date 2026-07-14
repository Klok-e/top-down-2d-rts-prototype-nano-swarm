//! Behavior tests for Logistics Corridor route-cost bias.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{DirectMovementComponent, HaulerAssignment, HaulerRoute, OwnerSwarm},
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_gather_haul()
}

fn own_for_player(app: &mut App, entities: &[Entity]) {
    let swarm = common::spawn_swarm_at(app, Vec2::ZERO);
    for entity in entities {
        app.world_mut()
            .entity_mut(*entity)
            .insert(OwnerSwarm(swarm));
    }
}

fn paint_corridor(app: &mut App, cell: IVec2) {
    assert!(
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(cell, IntentKind::Corridor)
    );
}

fn route_visits_row(route: &HaulerRoute, y: i32) -> bool {
    route
        .waypoints
        .iter()
        .any(|point| top_down_2d_rts_prototype_nano_swarm::nanobot::world_to_cell(*point).y == y)
}

#[test]
fn hauler_uses_route_system_without_corridor_paint() {
    let mut app = build_app();
    let hauler_pos = Vec2::new(0.0, 0.0);
    let source_pos = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
    let sink_pos = Vec2::new(3.5 * ZONE_BLOCK_SIZE, 0.0);
    let _source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let _sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    own_for_player(&mut app, &[_source, _sink]);
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);

    app.update();

    let route = app
        .world()
        .entity(hauler)
        .get::<HaulerRoute>()
        .expect("hauler source leg should use a route even without corridor paint");
    assert!(
        route.waypoints.iter().all(|point| {
            top_down_2d_rts_prototype_nano_swarm::nanobot::world_to_cell(*point).y == 0
        }),
        "unpainted route should follow the shortest row; got {:?}",
        route.waypoints
    );
}

#[test]
fn corridor_only_intent_does_not_create_hauling_job() {
    let mut app = build_app();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));
    paint_corridor(&mut app, IVec2::new(0, 0));

    for _ in 0..5 {
        app.update();
    }

    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
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
        app.world().entity(hauler).get::<HaulerRoute>().is_none(),
        "corridor must not create a route without a logistics leg"
    );
}

#[test]
fn route_follower_reissues_current_waypoint_when_timeout_strips_dmc() {
    let mut app = build_app();
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 0.0));
    let waypoint = Vec2::new(ZONE_BLOCK_SIZE, 0.0);
    app.world_mut()
        .entity_mut(hauler)
        .insert(HaulerRoute::new(vec![waypoint], 0.0));

    app.update();

    let dmc = app
        .world()
        .entity(hauler)
        .get::<DirectMovementComponent>()
        .expect("route follower should restore movement to the active waypoint");
    assert_eq!(dmc.xy, waypoint);
}

#[test]
fn leg_selection_uses_corridor_biased_route_cost() {
    let mut app = build_app();
    let hauler_pos = Vec2::new(0.0, 0.0);
    let near_sink_pos = Vec2::new(0.0, 2.0 * ZONE_BLOCK_SIZE);
    let corridor_sink_pos = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
    let source = common::spawn_stockpile(&mut app, hauler_pos, 1000, 1000);
    let near_sink = common::spawn_sink_stockpile(&mut app, near_sink_pos, 0, 1000);
    let corridor_sink = common::spawn_sink_stockpile(&mut app, corridor_sink_pos, 0, 1000);
    own_for_player(&mut app, &[source, near_sink, corridor_sink]);
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);
    paint_corridor(&mut app, IVec2::new(2, 0));
    paint_corridor(&mut app, IVec2::new(3, 0));

    app.update();

    let assignment = app
        .world()
        .entity(hauler)
        .get::<HaulerAssignment>()
        .expect("hauler should choose a valid logistics leg");
    assert_eq!(assignment.source, source);
    assert_eq!(
        assignment.sink, corridor_sink,
        "route cost should beat plain physical distance when corridor discount outweighs detour"
    );
    assert_ne!(assignment.sink, near_sink);
}

#[test]
fn source_leg_route_can_take_physically_longer_corridor_detour() {
    let mut app = build_app();
    let hauler_pos = Vec2::new(0.0, 0.0);
    let source_pos = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
    let sink_pos = Vec2::new(3.5 * ZONE_BLOCK_SIZE, 0.0);
    let _source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let _sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    own_for_player(&mut app, &[_source, _sink]);
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);

    for cell in [
        IVec2::new(0, 1),
        IVec2::new(1, 1),
        IVec2::new(2, 1),
        IVec2::new(3, 1),
    ] {
        paint_corridor(&mut app, cell);
    }

    app.update();

    let route = app
        .world()
        .entity(hauler)
        .get::<HaulerRoute>()
        .expect("hauler should have a source-leg route");
    assert!(
        route_visits_row(route, 1),
        "strong corridor detour should bias route; got {:?}",
        route.waypoints
    );
}

#[test]
fn route_stays_stable_after_corridor_paint_changes() {
    let mut app = build_app();
    let hauler_pos = Vec2::new(0.0, 0.0);
    let source_pos = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
    let sink_pos = Vec2::new(3.5 * ZONE_BLOCK_SIZE, 0.0);
    let _source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let _sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    own_for_player(&mut app, &[_source, _sink]);
    let hauler = common::spawn_hauler_at(&mut app, hauler_pos);

    let cells = [
        IVec2::new(0, 1),
        IVec2::new(1, 1),
        IVec2::new(2, 1),
        IVec2::new(3, 1),
    ];
    for cell in cells {
        paint_corridor(&mut app, cell);
    }

    app.update();
    let before = app
        .world()
        .entity(hauler)
        .get::<HaulerRoute>()
        .expect("hauler should have a route")
        .waypoints
        .clone();

    for cell in cells {
        assert!(
            app.world_mut()
                .resource_mut::<IntentGrid>()
                .erase(cell, IntentKind::Corridor)
        );
    }
    app.update();

    let after = app
        .world()
        .entity(hauler)
        .get::<HaulerRoute>()
        .expect("active logistics leg should keep its route")
        .waypoints
        .clone();
    assert_eq!(after, before);
}

#[test]
fn carry_leg_uses_corridor_biased_route_to_sink() {
    let mut app = build_app();
    let source_pos = Vec2::new(0.0, 0.0);
    let sink_pos = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
    let source = common::spawn_stockpile(&mut app, source_pos, 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, sink_pos, 0, 1000);
    own_for_player(&mut app, &[source, sink]);
    let hauler = common::spawn_hauler_at(&mut app, source_pos);

    for cell in [
        IVec2::new(0, 1),
        IVec2::new(1, 1),
        IVec2::new(2, 1),
        IVec2::new(3, 1),
    ] {
        paint_corridor(&mut app, cell);
    }
    app.world_mut()
        .entity_mut(hauler)
        .insert(HaulerAssignment { source, sink });

    for _ in 0..8 {
        app.update();
    }

    let route = app
        .world()
        .entity(hauler)
        .get::<HaulerRoute>()
        .expect("loaded hauler should have a carry-leg route");
    assert!(
        route_visits_row(route, 1),
        "carry leg should follow corridor-biased route; got {:?}",
        route.waypoints
    );
}
