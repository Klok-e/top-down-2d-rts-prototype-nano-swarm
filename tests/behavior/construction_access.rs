//! Automatic placement uses the same access and retry policy as completion.
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, OwnerSwarm, PlannedKind, PlannedStructure, Swarm, SwarmId,
        charger_auto_creation_system,
        construction_access::{CancelledSites, ConstructionAccess},
    },
};
#[path = "../common/mod.rs"]
mod common;

fn charger_plan(app: &mut App) -> Option<Vec2> {
    app.world_mut()
        .query::<(&PlannedStructure, &Transform)>()
        .iter(app.world())
        .find(|(plan, _)| plan.kind == PlannedKind::Charger)
        .map(|(_, transform)| transform.translation.truncate())
}

#[test]
fn automatic_construction_does_not_require_disconnected_friendly_networks_to_join() {
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
    // A full-height wall separates two friendly Stockpiles. The builder and
    // proposed Charger are in the right-hand network, which has usable space.
    for (center, half, owner) in [
        (Vec2::new(-36.0, 0.0), Vec2::new(36.0, 512.0), enemy),
        (Vec2::new(-252.0, 252.0), Vec2::splat(36.0), friendly),
        (Vec2::new(468.0, 252.0), Vec2::splat(36.0), friendly),
    ] {
        let entity = common::spawn_stockpile(&mut app, center, 0, 20);
        app.world_mut().entity_mut(entity).insert((
            Transform::from_translation(center.extend(0.0)).with_scale((half / 32.0).extend(1.0)),
            OwnerSwarm(owner),
        ));
    }
    for _ in 0..100 {
        app.update();
        if charger_plan(&mut app).is_some() {
            break;
        }
    }
    let chosen = charger_plan(&mut app).expect("existing disconnection must not veto a safe site");
    assert!(chosen.distance(Vec2::new(252.0, 252.0)) < 0.001);
}

#[test]
fn automatic_construction_may_close_an_enemy_only_connection() {
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(2, 2))
        .add_systems(Update, charger_auto_creation_system);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app.world_mut().spawn((Swarm {}, SwarmId(2))).id();
    common::spawn_worker_at(&mut app, Vec2::new(108.0, 108.0));
    let defender = common::spawn_defender_at(&mut app, Vec2::new(252.0, 252.0));
    app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::ZERO,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    // The sole passage connects enemy Stockpiles; no friendly endpoint depends on it.
    for (center, half) in [
        (Vec2::new(252.0, -166.0), Vec2::new(36.0, 346.0)),
        (Vec2::new(252.0, 418.0), Vec2::new(36.0, 94.0)),
        (Vec2::new(36.0, 252.0), Vec2::splat(36.0)),
        (Vec2::new(468.0, 252.0), Vec2::splat(36.0)),
    ] {
        let entity = common::spawn_stockpile(&mut app, center, 0, 20);
        app.world_mut().entity_mut(entity).insert((
            Transform::from_translation(center.extend(0.0)).with_scale((half / 32.0).extend(1.0)),
            OwnerSwarm(enemy),
        ));
    }
    for _ in 0..100 {
        app.update();
        if charger_plan(&mut app).is_some() {
            break;
        }
    }
    let chosen = charger_plan(&mut app).expect("enemy access is not protected");
    assert!(
        chosen.distance(Vec2::new(252.0, 252.0)) < 0.001,
        "the preferred site may seal the enemy passage: {chosen}"
    );
}

fn assert_planner_waits_for_builder(kind: PlannedKind) {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        GatherAssignment, NanobotType, ProductionPressure, ProductionPriority, SwarmMember,
        production_facility_auto_creation_system, sink_stockpile_demand_system,
        source_stockpile_demand_system,
    };
    for swarm in [SwarmId::PLAYER, SwarmId(2)] {
        let mut app = common::minimal_app();
        app.insert_resource(IntentGrid::new(2, 2));
        let owner = common::spawn_swarm_at(&mut app, Vec2::new(108.0, 108.0));
        app.world_mut().entity_mut(owner).insert(swarm);
        let worker = common::spawn_worker_at(&mut app, Vec2::new(-180.0, 108.0));
        app.world_mut()
            .entity_mut(worker)
            .insert(SwarmMember(swarm));
        let wall_owner = app.world_mut().spawn((Swarm {}, SwarmId(3))).id();
        let wall = common::spawn_stockpile(&mut app, Vec2::new(-36.0, 0.0), 0, 20);
        app.world_mut().entity_mut(wall).insert((
            Transform::from_xyz(-36.0, 0.0, 0.0).with_scale(Vec3::new(1.125, 16.0, 1.0)),
            OwnerSwarm(wall_owner),
        ));
        let intent = match kind {
            PlannedKind::Charger => IntentKind::Defend,
            PlannedKind::SourceStockpile => IntentKind::Gather,
            _ => IntentKind::Build,
        };
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint_owned(IVec2::ZERO, intent, Some(swarm));
        match kind {
            PlannedKind::Charger => {
                let defender = common::spawn_defender_at(&mut app, Vec2::new(252.0, 252.0));
                app.world_mut()
                    .entity_mut(defender)
                    .insert(SwarmMember(swarm));
                app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
                app.add_systems(Update, charger_auto_creation_system);
            }
            PlannedKind::SourceStockpile => {
                let deposit = common::spawn_deposit(
                    &mut app,
                    common::DepositFixture {
                        world_pos: Vec2::new(252.0, 252.0),
                        amount: 100,
                        capacity: 100,
                        radius: 32.0,
                    },
                );
                app.world_mut()
                    .entity_mut(worker)
                    .insert(GatherAssignment::new(IVec2::ZERO, deposit));
                app.add_systems(Update, source_stockpile_demand_system);
            }
            PlannedKind::SinkStockpile => {
                common::spawn_facility_at(&mut app, owner, Vec2::new(396.0, 252.0));
                app.add_systems(Update, sink_stockpile_demand_system);
            }
            PlannedKind::ProductionFacility => {
                let mut priority = ProductionPriority::new();
                priority.set_weight(NanobotType::Worker, 10);
                priority.set_weight(NanobotType::Hauler, 10);
                priority.set_weight(NanobotType::Defender, 10);
                app.insert_resource(priority)
                    .init_resource::<ProductionPressure>()
                    .add_systems(Update, production_facility_auto_creation_system);
            }
        }
        // The only Worker is west of a full-height wall; all painted sites
        // and demand are east of it. Deferred search must settle without a plan.
        for _ in 0..500 {
            app.update();
            assert_eq!(
                app.world_mut()
                    .query::<&PlannedStructure>()
                    .iter(app.world())
                    .count(),
                0,
                "{kind:?} must wait for a reachable builder in {swarm:?}"
            );
        }
        app.world_mut().despawn(wall);
        for _ in 0..200 {
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
        let (plan, plan_owner, transform) = app
            .world_mut()
            .query::<(&PlannedStructure, &OwnerSwarm, &Transform)>()
            .single(app.world())
            .expect("opening builder access must unblock planning");
        assert_eq!(plan.kind, kind);
        assert_eq!(plan_owner.0, owner);
        assert!((36.0..=468.0).contains(&transform.translation.x));
        assert!((36.0..=468.0).contains(&transform.translation.y));
        for coordinate in [transform.translation.x, transform.translation.y] {
            assert!(((coordinate - 36.0) / 72.0).fract().abs() < 0.001);
        }
    }
}

#[test]
fn source_planner_waits_for_builder_access_for_both_swarms() {
    assert_planner_waits_for_builder(PlannedKind::SourceStockpile);
}

#[test]
fn sink_planner_waits_for_builder_access_for_both_swarms() {
    assert_planner_waits_for_builder(PlannedKind::SinkStockpile);
}

#[test]
fn charger_planner_waits_for_builder_access_for_both_swarms() {
    assert_planner_waits_for_builder(PlannedKind::Charger);
}

#[test]
fn production_planner_waits_for_builder_access_for_both_swarms() {
    assert_planner_waits_for_builder(PlannedKind::ProductionFacility);
}

#[test]
fn automatic_construction_considers_both_passages_when_another_plan_closes_one() {
    for lower_passage_planned in [false, true] {
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
        // The wall has two 144-unit openings, at y=-108..36 and 180..324.
        // A centered 72-unit plan closes either opening to a 68-unit body.
        for (center, half, owner) in [
            (Vec2::new(252.0, -310.0), Vec2::new(36.0, 202.0), enemy),
            (Vec2::new(252.0, 108.0), Vec2::new(36.0, 72.0), enemy),
            (Vec2::new(252.0, 418.0), Vec2::new(36.0, 94.0), enemy),
            (Vec2::new(36.0, 252.0), Vec2::splat(36.0), friendly),
            (Vec2::new(468.0, 252.0), Vec2::splat(36.0), friendly),
        ] {
            let entity = common::spawn_stockpile(&mut app, center, 0, 20);
            app.world_mut().entity_mut(entity).insert((
                Transform::from_translation(center.extend(0.0))
                    .with_scale((half / 32.0).extend(1.0)),
                OwnerSwarm(owner),
            ));
        }
        if lower_passage_planned {
            let plan = common::spawn_planned_structure_of_kind_at_cell(
                &mut app,
                IVec2::new(0, -1),
                PlannedKind::SourceStockpile,
            );
            app.world_mut().entity_mut(plan).insert((
                Transform::from_xyz(252.0, -36.0, 0.0).with_scale(Vec3::new(1.125, 1.125, 1.0)),
                OwnerSwarm(friendly),
            ));
        }
        for _ in 0..200 {
            app.update();
            if charger_plan(&mut app).is_some() {
                break;
            }
        }
        let chosen = charger_plan(&mut app).expect("demand must find a safe Charger site");
        if lower_passage_planned {
            assert!(
                chosen.distance(Vec2::new(252.0, 252.0)) >= 144.0,
                "both plans together must not seal the friendly connection: {chosen}"
            );
        } else {
            assert!(
                chosen.distance(Vec2::new(252.0, 252.0)) < 0.001,
                "closing one passage is safe while the second stays open: {chosen}"
            );
        }
    }
}

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
