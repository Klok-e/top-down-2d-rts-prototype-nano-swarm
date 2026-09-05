//! Behavior tests for Logistics Corridor route-cost bias.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{DirectMovementComponent, HaulerAssignment, OwnerSwarm},
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
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(68.0, 0.0));
    paint_corridor(&mut app, IVec2::new(1, 0));
    paint_corridor(&mut app, IVec2::new(2, 0));
    paint_corridor(&mut app, IVec2::new(3, 0));

    for _ in 0..120 {
        app.update();
        if app.world().get::<HaulerAssignment>(hauler).is_some() {
            break;
        }
    }

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
fn source_leg_moves_through_the_painted_corridor() {
    let mut app = build_app();
    let source = common::spawn_stockpile(&mut app, Vec2::new(1536.0, 256.0), 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(1792.0, 256.0), 0, 1000);
    own_for_player(&mut app, &[source, sink]);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(0.0, 256.0));
    for x in 0..4 {
        paint_corridor(&mut app, IVec2::new(x, 1));
    }
    let mut entered_corridor = false;
    let mut furthest_y = 0.0_f32;
    for _ in 0..500 {
        app.update();
        let position = app
            .world()
            .entity(hauler)
            .get::<Transform>()
            .unwrap()
            .translation
            .truncate();
        entered_corridor |= position.y >= 512.0;
        furthest_y = furthest_y.max(position.y);
        if position.x > 1200.0 {
            break;
        }
    }
    assert!(
        entered_corridor,
        "hauler should physically use the discounted corridor on the source leg; furthest y={furthest_y}"
    );
}

#[test]
fn carry_leg_moves_through_the_painted_corridor() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, LogisticsReservation},
        resources::ResourceKind,
    };
    let mut app = build_app();
    let source = common::spawn_stockpile(&mut app, Vec2::new(0.0, 256.0), 1000, 1000);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(1536.0, 256.0), 0, 1000);
    own_for_player(&mut app, &[source, sink]);
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(68.0, 256.0));
    app.world_mut().entity_mut(hauler).insert((
        HaulerAssignment { source, sink },
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 20,
        },
        LogisticsReservation::new(source, sink, ResourceKind::Minerals, 20),
    ));
    for x in 0..4 {
        paint_corridor(&mut app, IVec2::new(x, 1));
    }
    let mut entered_corridor = false;
    let mut furthest_y = 0.0_f32;
    for _ in 0..500 {
        app.update();
        let position = app
            .world()
            .entity(hauler)
            .get::<Transform>()
            .unwrap()
            .translation
            .truncate();
        entered_corridor |= position.y >= 512.0;
        furthest_y = furthest_y.max(position.y);
        if position.x > 1200.0 {
            break;
        }
    }
    assert!(
        entered_corridor,
        "loaded hauler should physically use the discounted corridor on the delivery leg; furthest y={furthest_y}"
    );
}

#[test]
fn unreachable_pickup_does_not_commit_a_hauler_or_reserve_minerals() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::LogisticsReservation, resources::Stockpile,
    };
    let mut app = build_app();
    app.insert_resource(IntentGrid::new(4, 2));
    let source = common::spawn_stockpile(&mut app, Vec2::new(512.0, 0.0), 100, 100);
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(768.0, 0.0), 0, 100);
    own_for_player(&mut app, &[source, sink]);
    let wall = common::spawn_stockpile(&mut app, Vec2::new(256.0, 0.0), 0, 100);
    app.world_mut()
        .entity_mut(wall)
        .insert(Transform::from_xyz(256.0, 0.0, 0.0).with_scale(Vec3::new(1.0, 64.0, 1.0)));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::ZERO);

    app.update();

    assert!(
        app.world()
            .entity(hauler)
            .get::<HaulerAssignment>()
            .is_none()
    );
    assert!(
        app.world()
            .entity(hauler)
            .get::<LogisticsReservation>()
            .is_none()
    );
    assert_eq!(
        app.world()
            .entity(source)
            .get::<Stockpile>()
            .unwrap()
            .amount,
        100
    );
}

#[test]
fn erasing_corridor_preserves_active_leg_but_changes_the_next_leg() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Cargo, HaulerLoading},
        resources::Stockpile,
    };
    let mut twins = [build_app(), build_app()].map(|mut app| {
        let source = common::spawn_stockpile(&mut app, Vec2::new(1536.0, 256.0), 20, 20);
        let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(0.0, 256.0), 0, 20);
        own_for_player(&mut app, &[source, sink]);
        let hauler = common::spawn_hauler_at(&mut app, Vec2::new(-100.0, 256.0));
        for x in 0..4 {
            paint_corridor(&mut app, IVec2::new(x, 1));
        }
        for _ in 0..240 {
            let before = app.world().get::<Transform>(hauler).unwrap().translation;
            let travelling = app.world().get::<DirectMovementComponent>(hauler).is_some();
            app.update();
            if travelling
                && app
                    .world()
                    .get::<Transform>(hauler)
                    .unwrap()
                    .translation
                    .distance(before)
                    > 0.001
            {
                break;
            }
        }
        assert!(
            app.world()
                .entity(hauler)
                .get::<DirectMovementComponent>()
                .is_some()
        );
        assert!(
            app.world()
                .entity(hauler)
                .get::<Transform>()
                .unwrap()
                .translation
                .x
                > -100.0
        );
        (app, hauler, sink)
    });
    for x in 0..4 {
        assert!(
            twins[1]
                .0
                .world_mut()
                .resource_mut::<IntentGrid>()
                .erase(IVec2::new(x, 1), IntentKind::Corridor)
        );
    }

    let mut reached_source = false;
    let mut source_leg_entered_corridor = false;
    for _ in 0..600 {
        let positions = twins.each_mut().map(|(app, hauler, _)| {
            app.update();
            app.world()
                .entity(*hauler)
                .get::<Transform>()
                .unwrap()
                .translation
                .truncate()
        });
        assert!(
            positions[0].distance(positions[1]) < 0.001,
            "paint erasure must preserve the committed movement; positions={positions:?}"
        );
        source_leg_entered_corridor |= positions[0].y >= 512.0;
        if twins[0]
            .0
            .world()
            .entity(twins[0].1)
            .get::<HaulerLoading>()
            .is_some()
        {
            assert!(
                twins[1]
                    .0
                    .world()
                    .entity(twins[1].1)
                    .get::<HaulerLoading>()
                    .is_some()
            );
            reached_source = true;
            break;
        }
    }
    assert!(
        reached_source,
        "both haulers finish the committed source leg"
    );
    assert!(
        source_leg_entered_corridor,
        "the preserved leg physically uses the former corridor"
    );

    let mut delivery_max_y = [256.0_f32; 2];
    let mut delivered = [false; 2];
    for _ in 0..600 {
        for (index, (app, hauler, sink)) in twins.iter_mut().enumerate() {
            app.update();
            let bot = app.world().entity(*hauler);
            if bot.get::<Cargo>().is_some() && bot.get::<HaulerLoading>().is_none() {
                delivery_max_y[index] =
                    delivery_max_y[index].max(bot.get::<Transform>().unwrap().translation.y);
            }
            delivered[index] = app.world().entity(*sink).get::<Stockpile>().unwrap().amount == 20;
        }
        if delivered.into_iter().all(|done| done) {
            break;
        }
    }
    assert!(
        delivered.into_iter().all(|done| done),
        "both next legs deliver all twenty minerals"
    );
    assert!(
        delivery_max_y[0] >= 512.0,
        "retained paint guides the next delivery leg: {delivery_max_y:?}"
    );
    assert!(
        delivery_max_y[1] < 400.0,
        "erased paint makes the next delivery leg take the ordinary direct route: {delivery_max_y:?}"
    );
}
