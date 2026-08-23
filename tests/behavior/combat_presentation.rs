use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::nanobot::{
    ActiveCombatPulses, CombatAppearance, CombatPresentationSettings, CombatVisualSnapshot,
    Nanobot, NanobotPresentationPlugin, NanobotType, NanobotVisual, ResolvedCombatFact,
    ResolvedCombatHit, Swarm, SwarmId, SwarmMember,
};

fn visual_child(world: &World, root: Entity) -> Entity {
    world
        .get::<Children>(root)
        .expect("presented nanobot needs children")
        .iter()
        .find(|child| world.get::<NanobotVisual>(*child).is_some())
        .expect("presented nanobot needs its public visual child")
}

#[test]
fn resolved_hit_drives_impact_and_returns_both_visuals_to_neutral() {
    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )))
        .add_plugins(NanobotPresentationPlugin);

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
