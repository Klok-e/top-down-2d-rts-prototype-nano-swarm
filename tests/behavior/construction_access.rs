//! Automatic placement uses the same access and retry policy as completion.
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, PlannedKind, PlannedStructure, SwarmId, charger_auto_creation_system,
        construction_access::{CancelledSites, ConstructionAccess},
    },
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn charger_demand_chooses_another_site_after_cancellation_without_layout_change() {
    let mut app = common::minimal_app();
    app.init_resource::<CancelledSites>()
        .add_systems(Update, charger_auto_creation_system);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-108.0, 180.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(256.0, 256.0));
    app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::ZERO,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    for _ in 0..100 {
        app.update();
        if app
            .world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .next()
            .is_some()
        {
            break;
        }
    }
    let (entity, first) = app
        .world_mut()
        .query_filtered::<(Entity, &Transform), With<PlannedStructure>>()
        .single(app.world())
        .map(|(e, t)| (e, *t))
        .unwrap();
    let layout = app
        .world_mut()
        .run_system_once(|access: ConstructionAccess| access.snapshot())
        .unwrap();
    app.world_mut().resource_mut::<CancelledSites>().record(
        &layout,
        entity,
        SwarmId::PLAYER,
        PlannedKind::Charger,
        &first,
    );
    app.world_mut().despawn(entity);
    for _ in 0..100 {
        app.update();
        if app
            .world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .next()
            .is_some()
        {
            break;
        }
    }
    let (_, second) = app
        .world_mut()
        .query_filtered::<(Entity, &Transform), With<PlannedStructure>>()
        .single(app.world())
        .map(|(e, t)| (e, *t))
        .unwrap();
    assert!(
        first.translation.distance(second.translation) > 1.0,
        "continued capacity demand must choose an alternative to the cancelled site"
    );
    for coordinate in [second.translation.x, second.translation.y] {
        assert!(
            ((coordinate - 36.0) / 72.0).fract().abs() < 0.001,
            "alternative site remains snapped to the physical grid"
        );
    }
    for _ in 0..100 {
        app.update();
        if app
            .world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .next()
            .is_some()
        {
            break;
        }
    }
    assert_eq!(
        app.world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .count(),
        1
    );
}

#[test]
fn automatic_charger_chooses_an_alternate_site_preserving_the_literal_wall_passage() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{InteractionRegion, OwnerSwarm, Swarm},
        navigation::{Navigation, Obstacle, RouteOutcome},
    };
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(2, 2))
        .add_systems(Update, charger_auto_creation_system);
    let friendly = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app.world_mut().spawn((Swarm {}, SwarmId(2))).id();
    common::spawn_worker_at(&mut app, Vec2::new(108.0, 108.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(252.0, 252.0));
    app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::ZERO,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let mut shapes = Vec::new();
    // Opponent wall pieces leave only y=180..324 open; closing its middle
    // leaves 36-unit gaps, too small for a 68-unit body.
    for (center, half, owner) in [
        (Vec2::new(252.0, -166.0), Vec2::new(36.0, 346.0), enemy),
        (Vec2::new(252.0, 418.0), Vec2::new(36.0, 94.0), enemy),
        (Vec2::new(36.0, 252.0), Vec2::splat(36.0), friendly),
        (Vec2::new(468.0, 252.0), Vec2::splat(36.0), friendly),
    ] {
        let entity = common::spawn_stockpile(&mut app, center, 0, 20);
        let transform =
            Transform::from_translation(center.extend(0.0)).with_scale((half / 32.0).extend(1.0));
        app.world_mut()
            .entity_mut(entity)
            .insert((transform, OwnerSwarm(owner)));
        shapes.push(Obstacle::structure(&transform));
    }
    for _ in 0..100 {
        app.update();
        if app
            .world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .next()
            .is_some()
        {
            break;
        }
    }
    let chosen = *app
        .world_mut()
        .query_filtered::<&Transform, With<PlannedStructure>>()
        .single(app.world())
        .expect("safe alternative remains in the painted zone");
    assert!(
        chosen
            .translation
            .truncate()
            .distance(Vec2::new(252.0, 252.0))
            >= 144.0,
        "nearest placement would seal the only friendly passage"
    );
    shapes.push(Obstacle::structure(&chosen));
    let navigation = Navigation::new(app.world().resource::<IntentGrid>(), shapes);
    let target = Transform::from_xyz(468.0, 252.0, 0.0).with_scale(Vec3::new(1.125, 1.125, 1.0));
    assert!(
        matches!(
            navigation.route_to_interaction(
                Vec2::new(108.0, 252.0),
                InteractionRegion::structure(&target),
                app.world().resource::<IntentGrid>(),
                SwarmId::PLAYER,
                false
            ),
            RouteOutcome::Found(_)
        ),
        "friendly stockpiles remain connected after the selected plan becomes solid"
    );
}
