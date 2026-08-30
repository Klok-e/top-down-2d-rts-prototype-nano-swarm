use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActiveCombatPulses, ActiveNanobotDeathGhosts, ActiveStructureDeathGhosts, CombatPlugin,
        CombatPresentationSettings, DefenderResponse, Health, NanobotPresentationPlugin,
        OpponentSwarm, PlannedKind, Structure, Swarm, SwarmId, SwarmMember, completed_visual_color,
        nanobot_death_cleanup_system, world_to_cell,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn real_combat_fact_reaches_runtime_presentation_and_recovers() {
    let mut app = common::sim_app();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(CombatPlugin)
        .add_plugins(NanobotPresentationPlugin);

    let opponent = SwarmId(19);
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    app.world_mut()
        .spawn((Swarm {}, opponent, OpponentSwarm {}, Transform::default()));
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center + Vec2::new(-16.0, 0.0));
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(16.0, 0.0));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent));
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderResponse { target });
    let attacker_root = *app.world().get::<Transform>(attacker).unwrap();
    let target_root = *app.world().get::<Transform>(target).unwrap();

    app.update();

    assert_eq!(app.world().get::<Health>(target).unwrap().current, 90);
    let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
    let target_visual = common::nanobot_visual_child(app.world(), target);
    assert!(
        app.world()
            .get::<Transform>(attacker_visual)
            .unwrap()
            .translation
            .length()
            > 0.0,
    );
    assert!(
        app.world()
            .get::<Transform>(target_visual)
            .unwrap()
            .translation
            .length()
            > 0.0,
    );
    assert_ne!(
        app.world().get::<Sprite>(target_visual).unwrap().color,
        Color::WHITE,
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.recovery_duration + Duration::from_millis(1),
    ));
    app.update();

    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
    assert_eq!(
        app.world().get::<Transform>(attacker_visual),
        Some(&Transform::IDENTITY),
    );
    assert_eq!(
        app.world().get::<Transform>(target_visual),
        Some(&Transform::IDENTITY),
    );
    assert_eq!(
        app.world().get::<Sprite>(target_visual).unwrap().color,
        Color::WHITE,
    );
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));
}

#[test]
fn lethal_real_combat_removes_the_target_while_its_pulse_and_ghost_finish() {
    let mut app = common::sim_app();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(CombatPlugin)
        .add_plugins(NanobotPresentationPlugin)
        .add_systems(FixedLast, nanobot_death_cleanup_system);

    let opponent = SwarmId(19);
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    app.world_mut()
        .spawn((Swarm {}, opponent, OpponentSwarm {}, Transform::default()));
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center + Vec2::new(-16.0, 0.0));
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(16.0, 0.0));
    app.world_mut().entity_mut(target).insert((
        SwarmMember::new(opponent),
        Health {
            current: 10,
            max: 100,
        },
    ));
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderResponse { target });
    let attacker_root = *app.world().get::<Transform>(attacker).unwrap();

    app.update();

    assert!(!app.world().entities().contains(target));
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    let ghosts = app.world().resource::<ActiveNanobotDeathGhosts>();
    let ghost = ghosts.iter().next().unwrap();
    assert_eq!(ghost.victim.entity, target);

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5),
    ));
    app.update();

    assert_eq!(app.world().resource::<ActiveNanobotDeathGhosts>().len(), 1);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5) + Duration::from_millis(1),
    ));
    app.update();

    assert!(
        app.world()
            .resource::<ActiveNanobotDeathGhosts>()
            .is_empty()
    );
    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
    let returned_root = app.world().get::<Transform>(attacker).unwrap();
    assert_eq!(
        world_to_cell(returned_root.translation.truncate()),
        cell,
        "the released Defender should rejoin local staging",
    );
    assert_ne!(
        returned_root, &attacker_root,
        "a Defender released from lethal response duty should resume roaming",
    );
}

#[test]
fn surplus_defenders_leave_one_response_pulse_and_one_bounded_target_reaction() {
    let mut app = common::sim_app();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(CombatPlugin)
        .add_plugins(NanobotPresentationPlugin);

    let opponent = SwarmId(19);
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    app.world_mut()
        .spawn((Swarm {}, opponent, OpponentSwarm {}, Transform::default()));
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker_positions = [
        center + Vec2::new(-64.0, -32.0),
        center + Vec2::new(-64.0, 32.0),
        center + Vec2::new(0.0, -64.0),
    ];
    let attackers =
        attacker_positions.map(|position| common::spawn_defender_at(&mut app, position));
    let target = common::spawn_worker_at(&mut app, center);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent));
    for attacker in attackers {
        app.world_mut()
            .entity_mut(attacker)
            .insert(DefenderResponse { target });
    }
    let attacker_roots = attackers.map(|attacker| *app.world().get::<Transform>(attacker).unwrap());
    let target_root = *app.world().get::<Transform>(target).unwrap();

    app.update();

    assert_eq!(
        app.world().get::<Health>(target).unwrap().current,
        90,
        "presentation must not change one-claim combat damage",
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    let settings = *app.world().resource::<CombatPresentationSettings>();
    let target_visual = common::nanobot_visual_child(app.world(), target);
    let reaction = app
        .world()
        .get::<Transform>(target_visual)
        .unwrap()
        .translation
        .truncate();
    assert!(reaction.length() > 0.0);
    assert!(reaction.length() <= settings.recoil_distance + 0.001);
    assert_ne!(
        app.world().get::<Sprite>(target_visual).unwrap().color,
        Color::WHITE,
    );
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));
    let mut active_attacker_visuals = 0;
    for (attacker, root) in attackers.into_iter().zip(attacker_roots) {
        assert_eq!(app.world().get::<Transform>(attacker), Some(&root));
        let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
        if app
            .world()
            .get::<Transform>(attacker_visual)
            .unwrap()
            .translation
            .length()
            > 0.0
        {
            active_attacker_visuals += 1;
        }
    }
    assert_eq!(active_attacker_visuals, 1);
}

#[test]
fn real_support_structure_combat_flashes_in_place_and_recovers() {
    let mut app = common::sim_app();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(CombatPlugin)
        .add_plugins(NanobotPresentationPlugin);

    let opponent = SwarmId(19);
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    let opponent_entity = app
        .world_mut()
        .spawn((Swarm {}, opponent, OpponentSwarm {}, Transform::default()))
        .id();
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center + Vec2::new(-16.0, 0.0));
    let kind = PlannedKind::Charger;
    let structure_transform =
        Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(1.0))
            .with_rotation(Quat::from_rotation_z(0.31));
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        opponent_entity,
        kind,
        structure_transform,
        None,
    );
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderResponse { target: structure });
    let before_health = app.world().get::<Structure>(structure).unwrap().health;

    app.update();

    assert!(app.world().get::<Structure>(structure).unwrap().health < before_health);
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    assert_ne!(
        app.world().get::<Sprite>(structure).unwrap().color,
        completed_visual_color(),
    );
    assert_eq!(
        app.world().get::<Transform>(structure),
        Some(&structure_transform),
        "real support-structure combat must not move its gameplay root",
    );

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.recovery_duration + Duration::from_millis(1),
    ));
    app.update();

    assert_eq!(
        app.world().get::<Sprite>(structure).unwrap().color,
        completed_visual_color(),
    );
    assert_eq!(
        app.world().get::<Transform>(structure),
        Some(&structure_transform)
    );
}

#[test]
fn lethal_real_support_structure_combat_removes_gameplay_entity_while_effects_finish() {
    let mut app = common::sim_app();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(CombatPlugin)
        .add_plugins(NanobotPresentationPlugin);

    let opponent = SwarmId(19);
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    let opponent_entity = app
        .world_mut()
        .spawn((Swarm {}, opponent, OpponentSwarm {}, Transform::default()))
        .id();
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let attacker = common::spawn_defender_at(&mut app, center + Vec2::new(-16.0, 0.0));
    let kind = PlannedKind::ProductionFacility;
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        opponent_entity,
        kind,
        Transform::from_translation((center + Vec2::new(16.0, 0.0)).extend(1.0)),
        Some(1),
    );
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderResponse { target: structure });

    app.update();

    assert!(!app.world().entities().contains(structure));
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    let ghosts = app.world().resource::<ActiveStructureDeathGhosts>();
    let ghost = ghosts.iter().next().expect("structure death needs a ghost");
    assert_eq!(ghost.victim.entity, structure);
    assert!(ghost.ring_radius > 0.0);

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5),
    ));
    app.update();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5) + Duration::from_millis(1),
    ));
    app.update();

    assert!(
        app.world()
            .resource::<ActiveStructureDeathGhosts>()
            .is_empty()
    );
    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
}
