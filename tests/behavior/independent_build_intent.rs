//! Independent Build orders share physical space without sharing ownership.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        NanobotType, OwnerSwarm, PRODUCTION_PRESSURE_TICKS, PlannedKind, PlannedStructure,
        ProductionFacility, ProductionPriority, Swarm, SwarmId, SwarmMember,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn overlapping_build_orders_construct_separate_owned_facilities() {
    let mut app = common::sim_app_with_production_planned();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app
        .world_mut()
        .spawn((Swarm {}, SwarmId(7), Transform::default()))
        .id();
    let player_worker = common::spawn_worker_at(&mut app, Vec2::new(-512.0, -512.0));
    let enemy_worker = common::spawn_worker_at(&mut app, Vec2::new(-512.0, 512.0));
    app.world_mut()
        .entity_mut(enemy_worker)
        .insert(SwarmMember::new(SwarmId(7)));
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 10);
    priority.set_weight(NanobotType::Hauler, 10);
    app.insert_resource(priority);
    for cell in [IVec2::new(2, 0)] {
        for swarm in [SwarmId::PLAYER, SwarmId(7)] {
            app.world_mut()
                .resource_mut::<IntentGrid>()
                .paint(cell, IntentKind::Build, swarm);
        }
    }

    for _ in 0..PRODUCTION_PRESSURE_TICKS + 100 {
        app.update();
    }
    let plans = app
        .world_mut()
        .query::<(Entity, &PlannedStructure, &OwnerSwarm, &Transform)>()
        .iter(app.world())
        .filter(|(_, plan, _, _)| plan.kind == PlannedKind::ProductionFacility)
        .map(|(entity, _, owner, transform)| (entity, owner.0, transform.translation.truncate()))
        .collect::<Vec<_>>();
    assert_eq!(
        plans.len(),
        2,
        "both swarms need their own facility reservation"
    );
    assert!(plans.iter().any(|(_, owner, _)| *owner == player));
    assert!(plans.iter().any(|(_, owner, _)| *owner == enemy));
    let separation = (plans[0].2 - plans[1].2).abs();
    assert!(
        separation.x >= 72.0 || separation.y >= 72.0,
        "reservations must occupy separate physical space"
    );

    for (_, owner, center) in &plans {
        let worker = if *owner == player {
            player_worker
        } else {
            enemy_worker
        };
        app.world_mut()
            .entity_mut(worker)
            .insert(Transform::from_translation(
                (*center + Vec2::Y * 72.0).extend(0.0),
            ));
    }
    for _ in 0..220 {
        app.update();
        if plans
            .iter()
            .all(|(entity, _, _)| app.world().get::<ProductionFacility>(*entity).is_some())
        {
            break;
        }
    }
    for (entity, owner, _) in plans {
        assert!(
            app.world().get::<ProductionFacility>(entity).is_some(),
            "each swarm's Worker must finish its own reservation"
        );
        assert_eq!(
            app.world().get::<OwnerSwarm>(entity).unwrap().0,
            owner,
            "overlapping paint must preserve structure ownership through construction"
        );
    }
}

#[test]
fn enemy_only_build_does_not_supply_player_construction() {
    let mut app = common::sim_app_with_production();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    common::spawn_worker_at(&mut app, Vec2::new(-256.0, -256.0));
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 10);
    priority.set_weight(NanobotType::Hauler, 10);
    app.insert_resource(priority);
    let cell = IVec2::new(2, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, SwarmId(7));

    for _ in 0..PRODUCTION_PRESSURE_TICKS + 100 {
        app.update();
    }
    assert_eq!(
        app.world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .count(),
        0
    );

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, SwarmId::PLAYER);
    for _ in 0..100 {
        app.update();
    }
    let plans = app
        .world_mut()
        .query::<(&PlannedStructure, &OwnerSwarm)>()
        .iter(app.world())
        .filter(|(plan, _)| plan.kind == PlannedKind::ProductionFacility)
        .map(|(_, owner)| owner.0)
        .collect::<Vec<_>>();
    assert_eq!(
        plans,
        vec![player],
        "adding the player's own order enables its construction on enemy-painted space"
    );
    assert!(
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .unwrap()
            .has_owned(IntentKind::Build, SwarmId(7))
    );
}

#[test]
fn enemy_facility_blocks_its_footprint_but_not_the_rest_of_shared_build_cell() {
    let mut app = common::sim_app_with_production();
    let player = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let enemy = app
        .world_mut()
        .spawn((Swarm {}, SwarmId(7), Transform::default()))
        .id();
    common::spawn_worker_at(&mut app, Vec2::new(-256.0, -256.0));
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Worker, 10);
    priority.set_weight(NanobotType::Hauler, 10);
    app.insert_resource(priority);
    let cell = IVec2::new(2, 0);
    for swarm in [SwarmId::PLAYER, SwarmId(7)] {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(cell, IntentKind::Build, swarm);
    }
    let enemy_facility =
        common::spawn_facility_at(&mut app, enemy, common::cell_world_center(cell));
    let occupied = app
        .world()
        .get::<Transform>(enemy_facility)
        .unwrap()
        .translation
        .truncate();

    for _ in 0..PRODUCTION_PRESSURE_TICKS + 100 {
        app.update();
    }

    let plans = app
        .world_mut()
        .query::<(&PlannedStructure, &OwnerSwarm, &Transform)>()
        .iter(app.world())
        .filter(|(plan, _, _)| plan.kind == PlannedKind::ProductionFacility)
        .map(|(_, owner, transform)| (owner.0, transform.translation.truncate()))
        .collect::<Vec<_>>();
    assert_eq!(
        plans.len(),
        1,
        "the free part of the shared Build cell must remain available"
    );
    assert_eq!(plans[0].0, player);
    let separation = (plans[0].1 - occupied).abs();
    assert!(
        separation.x >= 72.0 || separation.y >= 72.0,
        "the new reservation must not overlap the enemy facility footprint"
    );
    assert_eq!(
        app.world().get::<OwnerSwarm>(enemy_facility).unwrap().0,
        enemy
    );
}
