use std::time::Duration;

use approx::assert_abs_diff_eq;
use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};

#[path = "../common/mod.rs"]
mod common;

use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        ActiveCombatDecorations, ActiveCombatPulses, ActiveNanobotDeathGhosts,
        ActiveStructureDeathGhosts, CombatAppearance, CombatPresentationSettings,
        CombatVisualSnapshot, Nanobot, NanobotPresentationPlugin, NanobotSprites, NanobotType,
        PlannedKind, ResolvedCombatDeath, ResolvedCombatFact, ResolvedCombatHit,
        StructureCombatAppearance, StructureKind, Swarm, SwarmId, SwarmMember,
        completed_visual_color,
    },
    structure_sprites::{StructureSprites, StructureVisual},
};

fn presentation_app() -> App {
    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )))
        .insert_resource(StructureSprites::from_single_handle(Handle::default()))
        .add_plugins(NanobotPresentationPlugin);
    app
}

fn assert_position(actual: Vec2, expected: Vec2) {
    assert_abs_diff_eq!(actual.x, expected.x, epsilon = 0.01);
    assert_abs_diff_eq!(actual.y, expected.y, epsilon = 0.01);
}

fn assert_translation(actual: Vec3, expected: Vec3) {
    assert_abs_diff_eq!(actual.x, expected.x, epsilon = 0.01);
    assert_abs_diff_eq!(actual.y, expected.y, epsilon = 0.01);
    assert_abs_diff_eq!(actual.z, expected.z, epsilon = 0.01);
}

fn assert_color(actual: Color, expected: Color) {
    let actual = actual.to_srgba();
    let expected = expected.to_srgba();
    assert_abs_diff_eq!(actual.red, expected.red, epsilon = 1e-5);
    assert_abs_diff_eq!(actual.green, expected.green, epsilon = 1e-5);
    assert_abs_diff_eq!(actual.blue, expected.blue, epsilon = 1e-5);
    assert_abs_diff_eq!(actual.alpha, expected.alpha, epsilon = 1e-5);
}

fn structure_snapshot(
    entity: Entity,
    position: Vec2,
    swarm: SwarmId,
    visual: StructureVisual,
) -> CombatVisualSnapshot {
    CombatVisualSnapshot {
        entity,
        position,
        swarm,
        appearance: CombatAppearance::Structure(StructureCombatAppearance {
            kind: StructureKind::Basic,
            visual: Some(visual),
        }),
    }
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

    let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
    let target_visual = common::nanobot_visual_child(app.world(), target);
    let settings = *app.world().resource::<CombatPresentationSettings>();
    assert!(settings.pulse_duration < Duration::from_millis(80));
    assert!(settings.recovery_duration < Duration::from_millis(250));
    assert!(settings.jab_distance < 16.0);
    assert!(settings.recoil_distance < 16.0);
    assert_abs_diff_eq!(settings.zoom_cutoff, 8.0, epsilon = 1e-5);

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
    assert_color(
        app.world().get::<Sprite>(attacker_visual).unwrap().color,
        Color::WHITE,
    );
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
    assert_position(pulses[0].start, attacker_position);
    assert_position(pulses[0].end, target_position);
    let pulse_color = pulses[0].color.to_srgba();
    assert!(pulse_color.blue > pulse_color.red);
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));

    let impact_facing_angle = attacker_pose.rotation.angle_between(Quat::IDENTITY);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.recovery_duration.mul_f32(0.5),
    ));
    app.update();

    let recovering_facing_angle = app
        .world()
        .get::<Transform>(attacker_visual)
        .unwrap()
        .rotation
        .angle_between(Quat::IDENTITY);
    assert!(recovering_facing_angle > 0.0);
    assert!(recovering_facing_angle < impact_facing_angle);

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
    assert_color(
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
    assert_eq!(ghost.victim.entity, victim_snapshot.entity);
    assert_position(ghost.victim.position, victim_snapshot.position);
    assert_eq!(ghost.victim.swarm, victim_snapshot.swarm);
    assert_eq!(ghost.victim.appearance, victim_snapshot.appearance);
    assert_translation(
        ghost.transform.translation,
        victim_position.extend(GAMEPLAY_SPRITE_Z),
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
fn structure_hit_flashes_in_place_with_attacker_jab_and_pulse_then_recovers() {
    let mut app = presentation_app();
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));

    let attacker_position = Vec2::new(-32.0, 0.0);
    let target_position = Vec2::new(32.0, 0.0);
    let attacker = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(attacker_position.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id();
    let target_transform = Transform::from_translation(target_position.extend(GAMEPLAY_SPRITE_Z))
        .with_rotation(Quat::from_rotation_z(0.37))
        .with_scale(Vec3::new(1.25, 0.8, 1.0));
    let neutral_color = completed_visual_color();
    let visual = StructureVisual::completed(PlannedKind::Charger);
    let sprite = Sprite {
        color: neutral_color,
        ..default()
    };
    let target = app
        .world_mut()
        .spawn((sprite, target_transform, visual))
        .id();
    app.update();
    let attacker_visual = common::nanobot_visual_child(app.world(), attacker);

    app.world_mut()
        .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
            attacker: CombatVisualSnapshot {
                entity: attacker,
                position: attacker_position,
                swarm: SwarmId::PLAYER,
                appearance: CombatAppearance::Nanobot(NanobotType::Defender),
            },
            target: structure_snapshot(target, target_position, SwarmId(11), visual),
            damage: 5,
            target_destroyed: false,
        }));
    app.update();

    assert_ne!(
        app.world().get::<Transform>(attacker_visual),
        Some(&Transform::IDENTITY),
        "the attacking Defender keeps the established jab",
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    assert_ne!(
        app.world().get::<Sprite>(target).unwrap().color,
        neutral_color,
        "the structure flashes at impact",
    );
    assert_eq!(
        app.world().get::<Transform>(target),
        Some(&target_transform),
        "structure impact must not recoil, rotate, or squash the gameplay root",
    );

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.recovery_duration + Duration::from_millis(1),
    ));
    app.update();

    assert_color(
        app.world().get::<Sprite>(target).unwrap().color,
        neutral_color,
    );
    assert_eq!(
        app.world().get::<Transform>(target),
        Some(&target_transform)
    );
    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
}

#[test]
fn simultaneous_structure_hits_keep_every_pulse_with_one_ghost_and_ring_then_expire() {
    let mut app = presentation_app();
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    app.world_mut()
        .resource_mut::<CombatPresentationSettings>()
        .max_decorative_effects = 0;

    let victim_position = Vec2::new(32.0, 0.0);
    let victim = app.world_mut().spawn_empty().id();
    assert!(app.world_mut().despawn(victim));
    let visual = StructureVisual::completed(PlannedKind::ProductionFacility);
    let victim_snapshot = structure_snapshot(victim, victim_position, SwarmId(11), visual);
    for attacker_position in [Vec2::new(-32.0, -8.0), Vec2::new(-32.0, 8.0)] {
        let attacker = app.world_mut().spawn_empty().id();
        app.world_mut()
            .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
                attacker: CombatVisualSnapshot {
                    entity: attacker,
                    position: attacker_position,
                    swarm: SwarmId::PLAYER,
                    appearance: CombatAppearance::Nanobot(NanobotType::Defender),
                },
                target: victim_snapshot,
                damage: 5,
                target_destroyed: true,
            }));
    }
    app.world_mut()
        .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
            victim: victim_snapshot,
        }));
    app.update();

    assert_eq!(
        app.world().resource::<ActiveCombatPulses>().len(),
        2,
        "every contributing hit keeps its primary pulse",
    );
    assert!(
        app.world().resource::<ActiveCombatDecorations>().is_empty(),
        "the decorative cap must not affect destruction signals",
    );
    assert!(
        app.world()
            .resource::<ActiveNanobotDeathGhosts>()
            .is_empty()
    );
    let structure_ghosts = app.world().resource::<ActiveStructureDeathGhosts>();
    let ghosts = structure_ghosts.iter().collect::<Vec<_>>();
    let [ghost] = ghosts.as_slice() else {
        panic!("one destroyed structure needs one ghost and one ring: {ghosts:?}");
    };
    assert_eq!(ghost.victim.entity, victim_snapshot.entity);
    assert_position(ghost.victim.position, victim_snapshot.position);
    assert_eq!(ghost.victim.swarm, victim_snapshot.swarm);
    assert_eq!(ghost.victim.appearance, victim_snapshot.appearance);
    assert_translation(
        ghost.transform.translation,
        victim_position.extend(GAMEPLAY_SPRITE_Z),
    );
    assert_eq!(
        ghost.image,
        app.world()
            .resource::<StructureSprites>()
            .production_facility,
    );
    assert!(ghost.ring_radius > 0.0);
    assert!(ghost.ring_color.to_srgba().alpha > 0.0);
    let initial_radius = ghost.ring_radius;
    let initial_alpha = ghost.color.to_srgba().alpha;
    let _ = ghosts;
    let _ = structure_ghosts;

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5),
    ));
    app.update();

    let structure_ghosts = app.world().resource::<ActiveStructureDeathGhosts>();
    let ghost = structure_ghosts.iter().next().unwrap();
    assert!(
        ghost.ring_radius > initial_radius,
        "the destruction ring expands"
    );
    assert!(
        ghost.color.to_srgba().alpha < initial_alpha,
        "the ghost fades"
    );

    app.insert_resource(TimeUpdateStrategy::ManualDuration(settings.death_duration));
    app.update();

    assert!(
        app.world()
            .resource::<ActiveStructureDeathGhosts>()
            .is_empty()
    );
    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
}

#[test]
fn structure_effects_hide_at_tactical_zoom_and_expire_while_hidden() {
    let mut app = presentation_app();
    let camera = app
        .world_mut()
        .spawn(CameraZoom2d {
            zoom: 7.99,
            ..default()
        })
        .id();
    let neutral_color = completed_visual_color();
    let visual = StructureVisual::completed(PlannedKind::SinkStockpile);
    let sprite = Sprite {
        color: neutral_color,
        ..default()
    };
    let live_structure = app
        .world_mut()
        .spawn((sprite, Transform::default(), visual))
        .id();
    let destroyed_structure = app.world_mut().spawn_empty().id();
    assert!(app.world_mut().despawn(destroyed_structure));
    let live_snapshot = structure_snapshot(live_structure, Vec2::ZERO, SwarmId(11), visual);
    let destroyed_snapshot = structure_snapshot(
        destroyed_structure,
        Vec2::new(32.0, 0.0),
        SwarmId(11),
        visual,
    );
    for target in [live_snapshot, destroyed_snapshot] {
        app.world_mut()
            .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
                attacker: CombatVisualSnapshot {
                    entity: Entity::PLACEHOLDER,
                    position: Vec2::new(-32.0, 0.0),
                    swarm: SwarmId::PLAYER,
                    appearance: CombatAppearance::Nanobot(NanobotType::Defender),
                },
                target,
                damage: 5,
                target_destroyed: target.entity == destroyed_structure,
            }));
    }
    app.world_mut()
        .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
            victim: destroyed_snapshot,
        }));
    app.update();

    assert_ne!(
        app.world().get::<Sprite>(live_structure).unwrap().color,
        neutral_color
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 2);
    assert_eq!(
        app.world().resource::<ActiveStructureDeathGhosts>().len(),
        1
    );

    app.world_mut()
        .get_mut::<CameraZoom2d>(camera)
        .unwrap()
        .zoom = 8.0;
    app.update();

    assert_color(
        app.world().get::<Sprite>(live_structure).unwrap().color,
        neutral_color,
    );
    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 2);
    assert_eq!(
        app.world().resource::<ActiveStructureDeathGhosts>().len(),
        1
    );

    let settings = *app.world().resource::<CombatPresentationSettings>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5),
    ));
    app.update();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration.mul_f32(0.5) + Duration::from_millis(1),
    ));
    app.update();

    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
    assert!(
        app.world()
            .resource::<ActiveStructureDeathGhosts>()
            .is_empty()
    );
    assert_color(
        app.world().get::<Sprite>(live_structure).unwrap().color,
        neutral_color,
    );
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

#[test]
fn primary_pulses_ignore_the_decorative_effect_budget() {
    let mut app = presentation_app();
    app.world_mut()
        .resource_mut::<CombatPresentationSettings>()
        .max_decorative_effects = 1;

    let target = app.world_mut().spawn_empty().id();
    for index in 0..3 {
        let attacker = app.world_mut().spawn_empty().id();
        app.world_mut()
            .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
                attacker: CombatVisualSnapshot {
                    entity: attacker,
                    position: Vec2::new(-32.0, index as f32 * 16.0),
                    swarm: SwarmId::PLAYER,
                    appearance: CombatAppearance::Nanobot(NanobotType::Defender),
                },
                target: CombatVisualSnapshot {
                    entity: target,
                    position: Vec2::new(32.0, 0.0),
                    swarm: SwarmId(11),
                    appearance: CombatAppearance::Nanobot(NanobotType::Worker),
                },
                damage: 10,
                target_destroyed: false,
            }));
    }

    app.update();

    assert_eq!(
        app.world().resource::<ActiveCombatPulses>().len(),
        3,
        "decorative limits must never remove primary combat signals",
    );
    assert_eq!(
        app.world().resource::<ActiveCombatDecorations>().len(),
        1,
        "secondary impact work must obey its configured bound",
    );
}

fn injected_reaction(
    attacker_positions: &[Vec2],
    spawn_order: &[usize],
    fact_order: &[usize],
) -> (Vec2, f32) {
    let mut app = presentation_app();
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    let opponent = SwarmId(11);
    app.world_mut()
        .spawn((Swarm {}, opponent, Transform::default()));
    app.world_mut()
        .resource_mut::<CombatPresentationSettings>()
        .max_decorative_effects = 1;

    let target_position = Vec2::ZERO;
    let target_root = Transform::from_translation(target_position.extend(GAMEPLAY_SPRITE_Z));
    let target = app
        .world_mut()
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            SwarmMember::new(opponent),
            target_root,
        ))
        .id();
    let mut attackers = vec![Entity::PLACEHOLDER; attacker_positions.len()];
    let mut attacker_roots = vec![Transform::IDENTITY; attacker_positions.len()];
    for &index in spawn_order {
        let root = Transform::from_translation(attacker_positions[index].extend(GAMEPLAY_SPRITE_Z));
        attackers[index] = app
            .world_mut()
            .spawn((
                Nanobot {},
                NanobotType::Defender,
                SwarmMember::new(SwarmId::PLAYER),
                root,
            ))
            .id();
        attacker_roots[index] = root;
    }
    app.update();

    for &index in fact_order {
        app.world_mut()
            .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
                attacker: CombatVisualSnapshot {
                    entity: attackers[index],
                    position: attacker_positions[index],
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
    }
    app.update();

    let settings = *app.world().resource::<CombatPresentationSettings>();
    assert_eq!(
        app.world().resource::<ActiveCombatPulses>().len(),
        attacker_positions.len(),
    );
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));
    for (index, attacker) in attackers.into_iter().enumerate() {
        assert_eq!(
            app.world().get::<Transform>(attacker),
            Some(&attacker_roots[index]),
        );
        let visual = common::nanobot_visual_child(app.world(), attacker);
        let jab = app.world().get::<Transform>(visual).unwrap().translation;
        assert!(jab.length() > 0.0);
        assert!(jab.length() <= settings.jab_distance + 0.001);
    }

    let target_visual = common::nanobot_visual_child(app.world(), target);
    let reaction = app
        .world()
        .get::<Transform>(target_visual)
        .unwrap()
        .translation
        .truncate();
    assert!(reaction.length() > 0.0);
    assert!(reaction.length() <= settings.recoil_distance + 0.001);
    let flash = app
        .world()
        .get::<Sprite>(target_visual)
        .unwrap()
        .color
        .to_linear();
    let flash_brightness = (flash.red + flash.green + flash.blue) / 3.0;
    (reaction, flash_brightness)
}

#[test]
fn simultaneous_hits_combine_into_one_deterministic_bounded_reaction() {
    let positions = [
        Vec2::new(-64.0, -32.0),
        Vec2::new(-64.0, 32.0),
        Vec2::new(0.0, -64.0),
    ];
    let (forward_reaction, crowded_flash_brightness) =
        injected_reaction(&positions, &[0, 1, 2], &[0, 1, 2]);
    let (reverse_reaction, reverse_flash_brightness) =
        injected_reaction(&positions, &[2, 1, 0], &[2, 1, 0]);
    let (_, single_flash_brightness) = injected_reaction(&positions[..1], &[0], &[0]);

    let expected_direction = Vec2::new(0.872_871_6, 0.487_950_03);
    assert!(
        forward_reaction.normalize().distance(expected_direction) < 0.001,
        "the target must recoil along the combined incoming direction",
    );
    assert!(
        forward_reaction.distance(reverse_reaction) < 0.001,
        "reaction aggregation must not depend on spawn or fact order",
    );
    assert!((crowded_flash_brightness - reverse_flash_brightness).abs() < 0.001);
    assert!(
        crowded_flash_brightness > single_flash_brightness,
        "simultaneous impacts must produce one brighter target flash",
    );
    assert!(
        crowded_flash_brightness
            <= 1.0 + CombatPresentationSettings::default().reaction_flash_cap + 0.001,
        "the combined flash must stay capped below full overexposure",
    );
}

#[test]
fn opposing_simultaneous_hits_keep_one_deterministic_recoil() {
    let positions = [Vec2::NEG_X * 64.0, Vec2::X * 64.0];
    let (forward_reaction, _) = injected_reaction(&positions, &[0, 1], &[0, 1]);
    let (reverse_reaction, _) = injected_reaction(&positions, &[1, 0], &[1, 0]);

    assert!(forward_reaction.length() > 0.0);
    assert!(forward_reaction.distance(reverse_reaction) < 0.001);
}

#[test]
fn combat_presentation_hides_at_tactical_zoom_and_expires_while_hidden() {
    let mut app = presentation_app();
    app.world_mut()
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()));
    let opponent = SwarmId(11);
    app.world_mut()
        .spawn((Swarm {}, opponent, Transform::default()));
    let camera = app
        .world_mut()
        .spawn(CameraZoom2d {
            zoom: 7.99,
            ..default()
        })
        .id();
    let attacker_position = Vec2::new(-32.0, 0.0);
    let target_position = Vec2::new(32.0, 0.0);
    let attacker_root = Transform::from_translation(attacker_position.extend(GAMEPLAY_SPRITE_Z));
    let target_root = Transform::from_translation(target_position.extend(GAMEPLAY_SPRITE_Z));
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
    let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
    let target_visual = common::nanobot_visual_child(app.world(), target);
    let settings = *app.world().resource::<CombatPresentationSettings>();
    let target_snapshot = CombatVisualSnapshot {
        entity: target,
        position: target_position,
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
            target: target_snapshot,
            damage: 10,
            target_destroyed: false,
        }));
    app.world_mut()
        .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath {
            victim: target_snapshot,
        }));
    app.update();

    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    assert!(settings.pulse_thickness >= 1.0);
    assert!(
        app.world()
            .get::<Transform>(attacker_visual)
            .unwrap()
            .translation
            .length()
            / 7.99
            >= settings.minimum_impact_screen_distance - 0.001,
    );
    assert!(
        app.world()
            .get::<Transform>(target_visual)
            .unwrap()
            .translation
            .length()
            / 7.99
            >= settings.minimum_impact_screen_distance - 0.001,
    );
    assert_eq!(app.world().resource::<ActiveNanobotDeathGhosts>().len(), 1);
    assert_eq!(app.world().resource::<ActiveCombatDecorations>().len(), 1);
    assert_ne!(
        app.world().get::<Transform>(attacker_visual),
        Some(&Transform::IDENTITY),
    );
    assert_ne!(
        app.world().get::<Transform>(target_visual),
        Some(&Transform::IDENTITY),
    );

    app.world_mut()
        .get_mut::<CameraZoom2d>(camera)
        .unwrap()
        .zoom = 8.0;
    app.update();

    assert_eq!(app.world().resource::<ActiveCombatPulses>().len(), 1);
    assert_eq!(app.world().resource::<ActiveNanobotDeathGhosts>().len(), 1);
    assert_eq!(app.world().resource::<ActiveCombatDecorations>().len(), 1);
    assert_eq!(
        app.world().get::<Transform>(attacker_visual),
        Some(&Transform::IDENTITY),
    );
    assert_eq!(
        app.world().get::<Transform>(target_visual),
        Some(&Transform::IDENTITY),
    );
    assert_color(
        app.world().get::<Sprite>(target_visual).unwrap().color,
        Color::WHITE,
    );
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(app.world().get::<Transform>(target), Some(&target_root));

    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        settings.death_duration + Duration::from_millis(1),
    ));
    app.update();
    app.world_mut()
        .get_mut::<CameraZoom2d>(camera)
        .unwrap()
        .zoom = 7.99;
    app.update();

    assert!(app.world().resource::<ActiveCombatPulses>().is_empty());
    assert!(
        app.world()
            .resource::<ActiveNanobotDeathGhosts>()
            .is_empty()
    );
    assert!(app.world().resource::<ActiveCombatDecorations>().is_empty());
    assert_eq!(
        app.world().get::<Transform>(attacker_visual),
        Some(&Transform::IDENTITY),
    );
    assert_eq!(
        app.world().get::<Transform>(target_visual),
        Some(&Transform::IDENTITY),
    );
}
