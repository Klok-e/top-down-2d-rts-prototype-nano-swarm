use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActiveCombatPulses, ActiveNanobotDeathGhosts, CombatPlugin, CombatPresentationSettings,
        DefendHold, Health, NanobotPresentationPlugin, NanobotVisual, OpponentSwarm, Swarm,
        SwarmId, SwarmMember, nanobot_death_cleanup_system,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn visual_child(world: &World, root: Entity) -> Entity {
    world
        .get::<Children>(root)
        .expect("presented combatant needs children")
        .iter()
        .find(|child| world.get::<NanobotVisual>(*child).is_some())
        .expect("presented combatant needs a visual child")
}

#[test]
fn real_combat_fact_reaches_runtime_presentation_and_recovers() {
    let mut app = common::sim_app_with_defend();
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
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefendHold { cell });
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(16.0, 0.0));
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent));
    let attacker_root = *app.world().get::<Transform>(attacker).unwrap();
    let target_root = *app.world().get::<Transform>(target).unwrap();

    app.update();

    assert_eq!(app.world().get::<Health>(target).unwrap().current, 90);
    let attacker_visual = visual_child(app.world(), attacker);
    let target_visual = visual_child(app.world(), target);
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
    let mut app = common::sim_app_with_defend();
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
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefendHold { cell });
    let target = common::spawn_worker_at(&mut app, center + Vec2::new(16.0, 0.0));
    app.world_mut().entity_mut(target).insert((
        SwarmMember::new(opponent),
        Health {
            current: 10,
            max: 100,
        },
    ));
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
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
}

#[test]
fn simultaneous_real_attacks_keep_every_pulse_and_one_bounded_target_reaction() {
    let mut app = common::sim_app_with_defend();
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
    let attackers = attacker_positions.map(|position| {
        let attacker = common::spawn_defender_at(&mut app, position);
        app.world_mut()
            .entity_mut(attacker)
            .insert(DefendHold { cell });
        attacker
    });
    let target = common::spawn_worker_at(&mut app, center);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(opponent));
    let attacker_roots = attackers.map(|attacker| *app.world().get::<Transform>(attacker).unwrap());
    let target_root = *app.world().get::<Transform>(target).unwrap();

    app.update();

    assert_eq!(
        app.world().get::<Health>(target).unwrap().current,
        70,
        "presentation must not change simultaneous combat damage",
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 3);
    let settings = *app.world().resource::<CombatPresentationSettings>();
    let target_visual = visual_child(app.world(), target);
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
    for (attacker, root) in attackers.into_iter().zip(attacker_roots) {
        assert_eq!(app.world().get::<Transform>(attacker), Some(&root));
        let attacker_visual = visual_child(app.world(), attacker);
        assert!(
            app.world()
                .get::<Transform>(attacker_visual)
                .unwrap()
                .translation
                .length()
                > 0.0,
        );
    }
}
