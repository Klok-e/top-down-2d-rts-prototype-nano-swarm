use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::{
    Nanobot, NanobotType, PRODUCTION_TICKS_PER_BOT, ProductionFacility,
    production_facility_work_system,
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn retained_output_satisfies_typed_demand_without_funding_a_duplicate() {
    use top_down_2d_rts_prototype_nano_swarm::{
        game_settings::GameSettings,
        intent::{IntentGrid, IntentKind},
        nanobot::{Commitment, PopulationDemandPlugin, ProductionPriority, SwarmId},
        resources::{ResourceKind, ResourceLedger},
    };
    let mut app = common::sim_app_with_production();
    app.add_plugins(PopulationDemandPlugin);
    app.world_mut().resource_mut::<GameSettings>().bot_speed = 0.;
    let mut priority = ProductionPriority::new();
    priority.set_weight(NanobotType::Defender, 100);
    app.insert_resource(priority);
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(2, 2),
        IntentKind::Corridor,
        SwarmId::PLAYER,
    );
    let retained = app
        .world_mut()
        .spawn((ProductionFacility::new(), Transform::from_xyz(36., 36., 0.)))
        .id();
    app.world_mut().entity_mut(retained).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::OwnerSwarm(swarm),
    );
    app.world_mut()
        .get_mut::<Transform>(retained)
        .unwrap()
        .scale = Vec3::new(1.125, 1.125, 1.);
    app.world_mut()
        .get_mut::<ProductionFacility>(retained)
        .unwrap()
        .input_amount = 20;
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        20,
    );
    for y in -1..=1 {
        for x in -1..=1 {
            if x != 0 || y != 0 {
                let bot = common::spawn_worker_at(
                    &mut app,
                    Vec2::new(36. + x as f32 * 72., 36. + y as f32 * 72.),
                );
                app.world_mut().entity_mut(bot).insert(Commitment::Working);
            }
        }
    }
    for _ in 0..250 {
        app.update();
    }
    let waiting = app.world().get::<ProductionFacility>(retained).unwrap();
    assert_eq!(waiting.current_target, Some(NanobotType::Defender));
    assert_eq!(waiting.progress, 120);
    let idle = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            Transform::from_xyz(756., 36., 0.),
        ))
        .id();
    app.world_mut().entity_mut(idle).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::OwnerSwarm(swarm),
    );
    app.world_mut()
        .get_mut::<ProductionFacility>(idle)
        .unwrap()
        .input_amount = 40;
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        40,
    );
    for _ in 0..250 {
        app.update();
    }
    let spare = app.world().get::<ProductionFacility>(idle).unwrap();
    assert_eq!(
        spare.current_target, None,
        "retained Defender already covers the one-tile reserve"
    );
    assert_eq!(spare.input_amount, 40);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        40
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<&NanobotType, With<Nanobot>>()
            .iter(app.world())
            .filter(|kind| **kind == NanobotType::Defender)
            .count(),
        0,
        "retained output counts toward demand but is not available in the world"
    );
}

#[test]
fn completed_output_waits_for_an_exterior_cell_and_releases_once() {
    let mut app = common::minimal_app();
    app.add_systems(Update, production_facility_work_system);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let mut facility = ProductionFacility::new();
    facility.current_target = Some(NanobotType::Worker);
    facility.progress = PRODUCTION_TICKS_PER_BOT - 1;
    let facility = app
        .world_mut()
        .spawn((
            facility,
            Transform::from_xyz(36.0, 36.0, 0.0).with_scale(Vec3::new(1.125, 1.125, 1.0)),
        ))
        .id();
    let mut blockers = Vec::new();
    for y in -1..=1 {
        for x in -1..=1 {
            if x != 0 || y != 0 {
                blockers.push(common::spawn_worker_at(
                    &mut app,
                    Vec2::new(36.0 + x as f32 * 72.0, 36.0 + y as f32 * 72.0),
                ));
            }
        }
    }
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        8,
        "finished output stays inside while every adjacent cell is occupied"
    );
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        Some(NanobotType::Worker)
    );
    let exit = *app.world().get::<Transform>(blockers[0]).unwrap();
    app.world_mut().despawn(blockers[0]);
    for _ in 0..5 {
        app.update();
    }
    let positions: Vec<_> = app
        .world_mut()
        .query_filtered::<&Transform, With<Nanobot>>()
        .iter(app.world())
        .map(|t| t.translation)
        .collect();
    assert_eq!(
        positions.len(),
        8,
        "one blocker removed and exactly one output released"
    );
    assert!(positions.contains(&exit.translation));
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        None
    );
}

#[test]
fn retained_output_keeps_one_funded_cycle_and_is_lost_with_its_facility() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{
            OwnerSwarm, ProductionPriority, SwarmId, SwarmMember,
            production_facility_pick_target_system,
        },
        resources::{ResourceKind, ResourceLedger},
    };
    for owner in [SwarmId::PLAYER, SwarmId(1)] {
        let mut app = common::minimal_app();
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Defender, 100);
        app.insert_resource(priority).add_systems(
            Update,
            (
                production_facility_pick_target_system,
                production_facility_work_system,
            )
                .chain(),
        );
        let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
        app.world_mut().entity_mut(swarm).insert(owner);
        let mut facility = ProductionFacility::new();
        facility.input_amount = 50;
        let facility = app
            .world_mut()
            .spawn((
                facility,
                OwnerSwarm(swarm),
                Transform::from_xyz(36., 36., 0.).with_scale(Vec3::new(1.125, 1.125, 1.)),
            ))
            .id();
        app.world_mut()
            .resource_mut::<ResourceLedger>()
            .add_for(owner, ResourceKind::Minerals, 50);
        for y in -1..=1 {
            for x in -1..=1 {
                if x != 0 || y != 0 {
                    let bot = common::spawn_worker_at(
                        &mut app,
                        Vec2::new(36. + x as f32 * 72., 36. + y as f32 * 72.),
                    );
                    app.world_mut()
                        .entity_mut(bot)
                        .insert(SwarmMember::new(owner));
                }
            }
        }
        for _ in 0..250 {
            app.update();
        }
        let retained = app.world().get::<ProductionFacility>(facility).unwrap();
        assert_eq!(retained.current_target, Some(NanobotType::Defender));
        assert_eq!(
            retained.input_amount, 30,
            "only one twenty-mineral cycle is charged while exits are blocked"
        );
        assert!(
            !retained.is_busy(),
            "exit waiting cannot justify production expansion"
        );
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(owner, ResourceKind::Minerals),
            30
        );
        app.world_mut().despawn(facility);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<Nanobot>>()
                .iter(app.world())
                .count(),
            8
        );
        assert_eq!(
            app.world()
                .resource::<ResourceLedger>()
                .total_for(owner, ResourceKind::Minerals),
            30,
            "destroying retained output cannot refund the paid production cost"
        );
    }
}

#[test]
fn oldest_finished_output_wins_a_shared_exit_even_when_created_later() {
    let mut app = common::minimal_app();
    app.add_systems(Update, production_facility_work_system);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let mut young = ProductionFacility::new();
    young.current_target = Some(NanobotType::Hauler);
    young.progress = PRODUCTION_TICKS_PER_BOT - 2;
    let young = app
        .world_mut()
        .spawn((
            young,
            Transform::from_xyz(180., 36., 0.).with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let mut old = ProductionFacility::new();
    old.current_target = Some(NanobotType::Defender);
    old.progress = PRODUCTION_TICKS_PER_BOT - 1;
    let old = app
        .world_mut()
        .spawn((
            old,
            Transform::from_xyz(36., 36., 0.).with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let mut shared = None;
    for y in [-36., 36., 108.] {
        for x in [-36., 36., 108., 180., 252.] {
            if y == 36. && (x == 36. || x == 180.) {
                continue;
            }
            let bot = common::spawn_worker_at(&mut app, Vec2::new(x, y));
            if x == 108. && y == 36. {
                shared = Some(bot);
            }
        }
    }
    for _ in 0..4 {
        app.update();
    }
    assert!(
        app.world()
            .get::<ProductionFacility>(old)
            .unwrap()
            .current_target
            .is_some()
    );
    app.world_mut().despawn(shared.unwrap());
    app.update();
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(old)
            .unwrap()
            .current_target,
        None
    );
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(young)
            .unwrap()
            .current_target,
        Some(NanobotType::Hauler)
    );
    let released = app
        .world_mut()
        .query::<(&NanobotType, &Transform)>()
        .iter(app.world())
        .find(|(kind, _)| **kind == NanobotType::Defender)
        .unwrap()
        .1;
    assert!((released.translation.truncate() - Vec2::new(108., 36.)).length() < 0.001);
}

#[test]
fn clearing_footprint_blocks_the_only_free_production_exit() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        PlannedKind, PlannedStructure, StructureClearing,
    };
    let mut app = common::minimal_app();
    app.add_systems(Update, production_facility_work_system);
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let mut facility = ProductionFacility::new();
    facility.current_target = Some(NanobotType::Worker);
    facility.progress = PRODUCTION_TICKS_PER_BOT;
    let facility = app
        .world_mut()
        .spawn((
            facility,
            Transform::from_xyz(36., 36., 0.).with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    for y in -1..=1 {
        for x in -1..=1 {
            if (x != 0 || y != 0) && (x != 1 || y != 0) {
                common::spawn_worker_at(
                    &mut app,
                    Vec2::new(36. + x as f32 * 72., 36. + y as f32 * 72.),
                );
            }
        }
    }
    let clearing = app
        .world_mut()
        .spawn((
            PlannedStructure::new(PlannedKind::Charger, IVec2::ZERO),
            StructureClearing::validated(Vec2::new(180., 36.), 0),
            Transform::from_xyz(108., 36., 0.).with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        7,
        "an empty clearing footprint is still barred to production"
    );
    assert!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target
            .is_some()
    );
    app.world_mut().despawn(clearing);
    app.update();
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Nanobot>>()
            .iter(app.world())
            .count(),
        8
    );
    assert!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target
            .is_none()
    );
}
