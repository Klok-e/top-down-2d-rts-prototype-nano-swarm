use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    nanobot::{
        ActiveCombatPulses, ActiveNanobotDeathGhosts, CombatAppearance, CombatPresentationSettings,
        CombatVisualSnapshot, Nanobot, NanobotPresentationPlugin, NanobotSprites, NanobotType,
        NanobotVisual, ResolvedCombatDeath, ResolvedCombatFact, ResolvedCombatHit, Swarm, SwarmId,
        SwarmMember,
    },
};

fn visual_child(world: &World, root: Entity) -> Entity {
    world
        .get::<Children>(root)
        .expect("presented nanobot needs children")
        .iter()
        .find(|child| world.get::<NanobotVisual>(*child).is_some())
        .expect("presented nanobot needs its public visual child")
}

fn presentation_app() -> App {
    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )))
        .add_plugins(NanobotPresentationPlugin);
    app
}

#[test]
fn resolved_hit_drives_impact_and_returns_both_visuals_to_neutral() {
    let mut app = presentation_app();

    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    let opponent = SwarmId(11);
    app.world_mut()
        .spawn((Swarm {}, opponent, Transform::default()));
    let attacker_position = Vec2::new(-32.0, 0.0);
    let target_position = Vec2::new(32.0, 0.0);
    let attacker_root = Transform::from_translation(attacker_position.extend(1.0))
        .with_rotation(Quat::from_rotation_z(0.4));
    let target_root = Transform::from_translation(target_position.extend(1.0))
        .with_rotation(Quat::from_rotation_z(-0.3));
    let attacker = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            attacker_root,
        ))
        .id();
    let target = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            SwarmMember::new(opponent),
            target_root,
        ))
        .id();
    app.update();

    let attacker_visual = visual_child(app.world(), attacker);
    let target_visual = visual_child(app.world(), target);
    let settings = *app.world().resource::<CombatPresentationSettings>();
    assert!(settings.pulse_duration < Duration::from_millis(80));
    assert!(settings.recovery_duration < Duration::from_millis(250));
    assert!(settings.jab_distance < 16.0);
    assert!(settings.recoil_distance < 16.0);
    assert_eq!(settings.zoom_cutoff, 8.0);

    app.world_mut()
        .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
            attacker: CombatVisualSnapshot {
                entity: attacker,
                position: attacker_position,
                swarm: SwarmId::PLAYER,
                appearance: CombatAppearance::Nanobot(NanobotType::Defender),
            },
            target: CombatVisualSnapshot {
                entity: target,
                position: target_position,
                swarm: opponent,
                appearance: CombatAppearance::Nanobot(NanobotType::Worker),
            },
            damage: 10,
            target_destroyed: false,
        }));
    app.update();

    let attacker_pose = *app.world().get::<Transform>(attacker_visual).unwrap();
    let target_pose = *app.world().get::<Transform>(target_visual).unwrap();
    assert!(attacker_pose.translation.length() > 0.0);
    assert!(attacker_pose.translation.length() <= settings.jab_distance + 0.001);
    assert!(attacker_pose.rotation != Quat::IDENTITY);
    assert!(target_pose.translation.length() > 0.0);
    assert!(target_pose.translation.length() <= settings.recoil_distance + 0.001);
    assert_ne!(
        app.world().get::<Sprite>(target_visual).unwrap().color,
        Color::WHITE,
        "the target must visibly flash at impact",
    );
    let pulses = app
        .world()
        .resource::<ActiveCombatPulses>()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(pulses.len(), 1);
    assert_eq!(pulses[0].start, attacker_position);
    assert_eq!(pulses[0].end, target_position);
    let pulse_color = pulses[0].color.to_srgba();
    assert!(pulse_color.blue > pulse_color.red);
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));

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
fn lethal_fact_sequence_keeps_the_final_pulse_with_a_collapsing_ghost_then_expires() {
    let mut app = presentation_app();

    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    let opponent = SwarmId(11);
    app.world_mut()
        .spawn((Swarm {}, opponent, Transform::default()));
    let attacker_position = Vec2::new(-32.0, 0.0);
    let victim_position = Vec2::new(32.0, 0.0);
    let attacker = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(attacker_position.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id();
    let victim = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            SwarmMember::new(opponent),
            Transform::from_translation(victim_position.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id();
    app.update();
    assert!(app.world_mut().despawn(victim));

    let victim_snapshot = CombatVisualSnapshot {
        entity: victim,
        position: victim_position,
        swarm: opponent,
        appearance: CombatAppearance::Nanobot(NanobotType::Worker),
    };
    app.world_mut()
        .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
            attacker: CombatVisualSnapshot {
                entity: attacker,
                position: attacker_position,
                swarm: SwarmId::PLAYER,
                appearance: CombatAppearance::Nanobot(NanobotType::Defender),
            },
            target: victim_snapshot,
            damage: 10,
            target_destroyed: true,
        }));
    app.world_mut()
        .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
            victim: victim_snapshot,
        }));
    app.update();

    let settings = *app.world().resource::<CombatPresentationSettings>();
    assert!(settings.death_duration >= Duration::from_millis(250));
    assert!(settings.death_duration <= Duration::from_millis(400));
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    let expected_image = app
        .world()
        .resource::<NanobotSprites>()
        .opponent_worker
        .clone();
    let ghosts = app.world().resource::<ActiveNanobotDeathGhosts>();
    let ghost_values = ghosts.iter().collect::<Vec<_>>();
    let [ghost] = ghost_values.as_slice() else {
        panic!("lethal combat needs exactly one presentation ghost");
    };
    assert_eq!(ghost.victim, victim_snapshot);
    assert_eq!(
        ghost.transform.translation,
        victim_position.extend(GAMEPLAY_SPRITE_Z)
    );
    assert_eq!(ghost.image, expected_image);
    assert_ne!(ghost.color, Color::WHITE, "the ghost starts flashed");
    let initial_transform = ghost.transform;
    let initial_alpha = ghost.color.to_srgba().alpha;
    drop(ghost_values);
    let _ = ghosts;

    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5),
    ));
    app.update();

    let ghosts = app.world().resource::<ActiveNanobotDeathGhosts>();
    let mid = ghosts.iter().next().unwrap();
    assert!(mid.transform.scale.y < initial_transform.scale.y);
    assert!(mid.transform.scale.x <= initial_transform.scale.x);
    assert!(mid.color.to_srgba().alpha < initial_alpha);

    app.insert_resource(TimeUpdateStrategy::ManualDuration(settings.death_duration));
    app.update();

    assert!(
        app.world()
            .resource::<ActiveNanobotDeathGhosts>()
            .is_empty()
    );
    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
}

#[test]
fn death_ghosts_reuse_every_nanobot_image_for_both_swarms() {
    let mut app = presentation_app();

    for (swarm_index, swarm) in [SwarmId::PLAYER, SwarmId(11)].into_iter().enumerate() {
        for (kind_index, kind) in NanobotType::ALL.into_iter().enumerate() {
            let victim = app.world_mut().spawn_empty().id();
            assert!(app.world_mut().despawn(victim));
            app.world_mut()
                .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
                    victim: CombatVisualSnapshot {
                        entity: victim,
                        position: Vec2::new(kind_index as f32 * 16.0, swarm_index as f32 * 16.0),
                        swarm,
                        appearance: CombatAppearance::Nanobot(kind),
                    },
                }));
        }
    }
    app.update();

    let sprites = app.world().resource::<NanobotSprites>().clone();
    let ghosts = app
        .world()
        .resource::<ActiveNanobotDeathGhosts>()
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(ghosts.len(), NanobotType::COUNT * 2);
    for ghost in ghosts {
        let CombatAppearance::Nanobot(kind) = ghost.victim.appearance else {
            panic!("nanobot death test emitted a non-nanobot appearance");
        };
        assert_eq!(
            ghost.image,
            sprites.handle(kind, !ghost.victim.swarm.is_player()),
        );
    }
}
