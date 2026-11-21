use std::time::Duration;

use approx::assert_abs_diff_eq;
use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, MainCamera,
    nanobot::{
        ActiveCombatBolts, ActiveCombatEffects, ActiveCombatIndicators, Charge, CombatAppearance,
        CombatIndicatorKind, CombatParticleKind, CombatVisualSnapshot, Health,
        NanobotPresentationPlugin, NanobotType, PlannedKind, PresentedNanobot, ResolvedCombatDeath,
        ResolvedCombatFact, ResolvedCombatHit, Structure, StructureCombatAppearance, StructureKind,
        Swarm, SwarmId, SwarmMember, completed_visual_color, reconcile_combat_presentation_tuning,
        swarm_presentation_color,
    },
    structure_sprites::{StructureSprites, StructureVisual},
    tuning::{PresentationTuning, Tuning},
};

#[path = "../common/mod.rs"]
mod common;

const OPPONENT: SwarmId = SwarmId(11);

fn presentation_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        bevy::time::TimePlugin,
        AssetPlugin::default(),
    ))
    .init_asset::<Image>()
    .insert_resource(StructureSprites::from_single_handle(Handle::default()))
    .add_plugins(NanobotPresentationPlugin);
    app
}

fn advance(app: &mut App, millis: u64) {
    advance_by(app, Duration::from_millis(millis));
}

fn advance_by(app: &mut App, delta: Duration) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(delta));
    app.update();
}

fn assert_position(actual: Vec2, expected: Vec2) {
    assert_abs_diff_eq!(actual.x, expected.x, epsilon = 0.01);
    assert_abs_diff_eq!(actual.y, expected.y, epsilon = 0.01);
}

fn nanobot_snapshot(
    entity: Entity,
    position: Vec2,
    swarm: SwarmId,
    kind: NanobotType,
) -> CombatVisualSnapshot {
    CombatVisualSnapshot {
        entity,
        position,
        swarm,
        appearance: CombatAppearance::Nanobot(kind),
    }
}

fn structure_snapshot(
    entity: Entity,
    position: Vec2,
    visual: StructureVisual,
) -> CombatVisualSnapshot {
    CombatVisualSnapshot {
        entity,
        position,
        swarm: OPPONENT,
        appearance: CombatAppearance::Structure(StructureCombatAppearance {
            kind: StructureKind::Basic,
            visual: Some(visual),
        }),
    }
}

fn emit_hit(
    app: &mut App,
    attacker: CombatVisualSnapshot,
    target: CombatVisualSnapshot,
    target_destroyed: bool,
) {
    app.world_mut()
        .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
            attacker,
            target,
            damage: 10,
            target_destroyed,
        }));
}

fn emit_death(app: &mut App, victim: CombatVisualSnapshot) {
    app.world_mut()
        .write_message(ResolvedCombatFact::Death(ResolvedCombatDeath { victim }));
}

#[test]
fn crossing_zoom_tiers_changes_bolts_impacts_fragments_and_indicators() {
    for (zoom, bolts_expected, sparks, flashes, fragments) in
        [(2.49, 1, 6, 0, 8), (2.5, 1, 0, 1, 4), (8.0, 0, 0, 0, 0)]
    {
        let mut app = presentation_app();
        app.world_mut().spawn((
            MainCamera,
            Transform::default(),
            Projection::Orthographic(OrthographicProjection {
                scale: zoom,
                area: Rect::from_center_size(Vec2::ZERO, Vec2::splat(1000.0)),
                ..OrthographicProjection::default_2d()
            }),
        ));
        let attacker = common::spawn_defender_at(&mut app, Vec2::new(-40.0, 0.0));
        let victim = app.world_mut().spawn_empty().id();
        let victim = nanobot_snapshot(victim, Vec2::new(40.0, 0.0), OPPONENT, NanobotType::Worker);
        advance(&mut app, 16);
        app.world_mut().get_mut::<Health>(attacker).unwrap().current -= 1;
        emit_hit(
            &mut app,
            nanobot_snapshot(
                attacker,
                Vec2::new(-40.0, 0.0),
                SwarmId::PLAYER,
                NanobotType::Defender,
            ),
            victim,
            true,
        );
        emit_death(&mut app, victim);

        advance(&mut app, 16);
        let bolts = app.world().resource::<ActiveCombatBolts>();
        assert_eq!(bolts.len(), bolts_expected, "zoom {zoom}");
        let indicators = app.world().resource::<ActiveCombatIndicators>();
        assert_eq!(
            indicators
                .iter()
                .any(|indicator| indicator.kind == CombatIndicatorKind::ChargeCore),
            zoom < 8.0,
            "charge core at zoom {zoom}"
        );
        assert_eq!(
            indicators
                .iter()
                .any(|indicator| matches!(indicator.kind, CombatIndicatorKind::HealthArc { .. })),
            zoom < 8.0,
            "health arc at zoom {zoom}"
        );

        advance(&mut app, 100);
        let effects = app.world().resource::<ActiveCombatEffects>();
        for (kind, count) in [
            (CombatParticleKind::ImpactSpark, sparks),
            (CombatParticleKind::ImpactFlash, flashes),
            (CombatParticleKind::Fragment, fragments),
        ] {
            assert_eq!(
                effects
                    .particles()
                    .filter(|particle| particle.kind == kind)
                    .count(),
                count,
                "zoom {zoom}, {kind:?}"
            );
        }
        assert_eq!(effects.death_pops().len(), bolts_expected, "zoom {zoom}");
        assert_eq!(effects.decal_count(), bolts_expected, "zoom {zoom}");
    }
}

#[test]
fn far_zoom_keeps_existing_decals_and_shows_one_fading_dot_per_nanobot_death() {
    let mut app = presentation_app();
    let camera = app
        .world_mut()
        .spawn((
            MainCamera,
            Transform::default(),
            Projection::Orthographic(OrthographicProjection {
                area: Rect::from_center_size(Vec2::ZERO, Vec2::splat(1000.0)),
                ..OrthographicProjection::default_2d()
            }),
        ))
        .id();
    advance(&mut app, 16);
    let first_victim = app.world_mut().spawn_empty().id();
    emit_death(
        &mut app,
        nanobot_snapshot(
            first_victim,
            Vec2::ZERO,
            SwarmId::PLAYER,
            NanobotType::Worker,
        ),
    );
    advance(&mut app, 16);
    assert_eq!(
        app.world().resource::<ActiveCombatEffects>().decal_count(),
        1
    );

    let mut camera_projection = app.world_mut().get_mut::<Projection>(camera).unwrap();
    let Projection::Orthographic(projection) = &mut *camera_projection else {
        panic!("expected an orthographic camera");
    };
    projection.scale = 12.0;
    advance(&mut app, 250);
    advance(&mut app, 150);
    let attacker = app.world_mut().spawn_empty().id();
    let second_victim = app.world_mut().spawn_empty().id();
    let position = Vec2::new(100.0, 25.0);
    let victim = nanobot_snapshot(second_victim, position, OPPONENT, NanobotType::Defender);
    emit_hit(
        &mut app,
        nanobot_snapshot(
            attacker,
            position - Vec2::X * 30.0,
            SwarmId::PLAYER,
            NanobotType::Defender,
        ),
        victim,
        true,
    );
    emit_death(&mut app, victim);
    advance(&mut app, 16);

    let effects = app.world().resource::<ActiveCombatEffects>();
    let [dot] = effects.casualty_flashes() else {
        panic!("expected one casualty dot")
    };
    assert_position(dot.position, position);
    assert_eq!(dot.color, swarm_presentation_color(OPPONENT));
    assert_abs_diff_eq!(dot.diameter_px(), 5.0);
    assert_abs_diff_eq!(dot.opacity(), 1.0);
    assert_eq!(effects.decal_count(), 1);

    advance(&mut app, 80);
    let dot = &app
        .world()
        .resource::<ActiveCombatEffects>()
        .casualty_flashes()[0];
    assert_abs_diff_eq!(dot.diameter_px(), 8.0);
    assert_abs_diff_eq!(dot.opacity(), 0.8);

    advance(&mut app, 200);
    advance(&mut app, 120);
    assert!(
        app.world()
            .resource::<ActiveCombatEffects>()
            .casualty_flashes()
            .is_empty()
    );
}

#[test]
fn combat_heat_records_hits_at_close_zoom_and_decays_without_further_hits() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::CombatHeat;

    let mut app = presentation_app();
    app.insert_resource(Time::<Fixed>::from_duration(Duration::from_millis(10)));
    let player = nanobot_snapshot(
        app.world_mut().spawn_empty().id(),
        Vec2::new(100.0, 100.0),
        SwarmId::PLAYER,
        NanobotType::Defender,
    );
    let opponent = nanobot_snapshot(
        app.world_mut().spawn_empty().id(),
        Vec2::new(200.0, 100.0),
        OPPONENT,
        NanobotType::Defender,
    );
    advance(&mut app, 0);
    emit_hit(&mut app, player, opponent, false);
    emit_hit(&mut app, opponent, player, false);
    advance(&mut app, 50);

    let initial_heat = app.world().resource::<CombatHeat>().total();
    // Both hits are captured in the first of this frame's five fixed ticks.
    assert_abs_diff_eq!(initial_heat, 1.9223, epsilon = 0.01);
    for _ in 0..14 {
        advance(&mut app, 50);
    }
    let half_life_heat = app.world().resource::<CombatHeat>().total();
    assert_abs_diff_eq!(half_life_heat / initial_heat, 0.5, epsilon = 0.01);

    for _ in 0..34 {
        advance(&mut app, 50);
    }
    let remaining_heat = app.world().resource::<CombatHeat>().total();
    assert!(remaining_heat < 0.1 * initial_heat);
}

#[test]
fn bolt_follows_moving_target_and_starts_impact_on_arrival() {
    for (attacking_swarm, target_swarm) in
        [(SwarmId::PLAYER, OPPONENT), (OPPONENT, SwarmId::PLAYER)]
    {
        let mut app = presentation_app();
        let start = Vec2::new(-40.0, 0.0);
        let target_position = Vec2::new(40.0, 0.0);
        let attacker = common::spawn_defender_at(&mut app, start);
        let target = common::spawn_worker_at(&mut app, target_position);
        app.world_mut()
            .entity_mut(attacker)
            .insert(SwarmMember::new(attacking_swarm));
        app.world_mut()
            .entity_mut(target)
            .insert(SwarmMember::new(target_swarm));
        advance(&mut app, 10);

        let attacker_root = *app.world().get::<Transform>(attacker).unwrap();
        let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
        let target_visual = common::nanobot_visual_child(app.world(), target);
        let target_is_neutral = |app: &App| {
            app.world().get::<Sprite>(target_visual).unwrap().color == Color::WHITE
                && app
                    .world()
                    .get::<Transform>(target_visual)
                    .unwrap()
                    .translation
                    == Vec3::ZERO
        };
        emit_hit(
            &mut app,
            nanobot_snapshot(
                attacker,
                start - Vec2::Y * 16.0,
                attacking_swarm,
                NanobotType::Defender,
            ),
            nanobot_snapshot(
                target,
                target_position - Vec2::Y * 16.0,
                target_swarm,
                NanobotType::Worker,
            ),
            false,
        );
        advance(&mut app, 10);

        assert!(
            target_is_neutral(&app),
            "a launch must leave the target pose untouched"
        );
        let bolts = app.world().resource::<ActiveCombatBolts>();
        let [bolt] = bolts.iter().collect::<Vec<_>>()[..] else {
            panic!("one hit should launch one bolt");
        };
        let launch_position = app
            .world()
            .get::<PresentedNanobot>(attacker)
            .unwrap()
            .sprite_translation()
            .truncate();
        assert_position(bolt.position, launch_position);
        let flight = bolt.flight_time;
        assert_abs_diff_eq!(
            flight.as_secs_f32(),
            (launch_position.distance(target_position) / 1600.0).clamp(0.04, 0.09),
            epsilon = 1e-5
        );
        assert_position(bolt.end, target_position);
        assert_eq!(bolt.color, swarm_presentation_color(attacking_swarm));
        let initial_jab = app
            .world()
            .get::<Transform>(attacker_visual)
            .unwrap()
            .translation;
        assert!(initial_jab.length() > 0.0, "the attacker jabs on launch");

        let target_destination = target_position + Vec2::Y * 32.0;
        app.world_mut()
            .get_mut::<Transform>(target)
            .unwrap()
            .translation = target_destination.extend(attacker_root.translation.z);
        let destination_root = *app.world().get::<Transform>(target).unwrap();
        let midpoint = flight / 2;
        advance_by(&mut app, midpoint);

        let bolt = app
            .world()
            .resource::<ActiveCombatBolts>()
            .iter()
            .next()
            .unwrap();
        assert_position(bolt.position, launch_position.lerp(target_destination, 0.5));
        assert_position(bolt.end, target_destination);
        advance_by(&mut app, flight - midpoint - Duration::from_millis(1));
        assert_eq!(app.world().resource::<ActiveCombatBolts>().len(), 1);
        assert!(
            target_is_neutral(&app),
            "a bolt still in flight must not flash or recoil the target"
        );
        assert_eq!(
            app.world()
                .resource::<ActiveCombatEffects>()
                .particles()
                .len(),
            0
        );

        advance(&mut app, 2);
        assert!(app.world().resource::<ActiveCombatBolts>().is_empty());
        assert_ne!(
            app.world().get::<Sprite>(target_visual).unwrap().color,
            Color::WHITE
        );
        let recoil = app
            .world()
            .get::<Transform>(target_visual)
            .unwrap()
            .translation
            .truncate();
        let recoil_limit = app
            .world()
            .resource::<Tuning>()
            .presentation
            .combat_recoil_distance;
        assert!((0.0..=recoil_limit + 0.001).contains(&recoil.length()) && recoil.length() > 0.0);
        assert!(
            recoil
                .normalize()
                .dot((target_destination - launch_position).normalize())
                > 0.999
        );
        let effects = app.world().resource::<ActiveCombatEffects>();
        let sparks: Vec<_> = effects
            .particles()
            .filter(|particle| particle.kind == CombatParticleKind::ImpactSpark)
            .collect();
        assert_eq!(sparks.len(), 6);
        assert!(
            sparks
                .iter()
                .all(|spark| spark.color == swarm_presentation_color(attacking_swarm))
        );
        assert!(
            sparks
                .iter()
                .all(|spark| (spark.lifetime_secs() - 0.18).abs() < 1e-5)
        );

        advance(&mut app, 90);
        let half_recoil = app
            .world()
            .get::<Transform>(target_visual)
            .unwrap()
            .translation
            .truncate();
        let recoil_overshoot = -half_recoil.dot(recoil.normalize()) / recoil.length();
        assert!((0.07..=0.11).contains(&recoil_overshoot));

        advance(&mut app, 160);
        assert!(target_is_neutral(&app));
        let attacker_pose = app.world().get::<Transform>(attacker_visual).unwrap();
        assert_eq!(attacker_pose.translation, Vec3::ZERO);
        let attacker_heading =
            (attacker_root.rotation * attacker_pose.rotation * Vec3::Y).truncate();
        assert_position(attacker_heading, Vec2::X);
        for _ in 0..4 {
            advance(&mut app, 160);
        }
        let settled = app.world().get::<PresentedNanobot>(attacker).unwrap();
        assert_position(
            (settled.rotation() * Vec3::Y).truncate(),
            (attacker_root.rotation * Vec3::Y).truncate(),
        );
        assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
        assert_eq!(
            app.world().get::<Transform>(target),
            Some(&destination_root)
        );
    }
}

#[test]
fn first_lethal_arrival_starts_one_pop_and_later_bolts_impact_its_position() {
    let mut app = presentation_app();
    let death_position = Vec2::new(32.0, 0.0);
    let victim = common::spawn_worker_at(&mut app, death_position);
    let attacker = |app: &mut App, distance| {
        let position = death_position - Vec2::X * distance;
        nanobot_snapshot(
            common::spawn_defender_at(app, position),
            position,
            SwarmId::PLAYER,
            NanobotType::Defender,
        )
    };
    let slow_shot = attacker(&mut app, 144.0);
    let fast_shot = attacker(&mut app, 64.0);
    advance(&mut app, 10);

    app.world_mut().despawn(victim);
    let victim = nanobot_snapshot(victim, death_position, OPPONENT, NanobotType::Worker);
    emit_hit(&mut app, slow_shot, victim, true);
    emit_hit(&mut app, fast_shot, victim, true);
    emit_death(&mut app, victim);
    advance(&mut app, 10);

    assert_eq!(app.world().resource::<ActiveCombatBolts>().len(), 2);
    advance(&mut app, 39);
    let bolts = app.world().resource::<ActiveCombatBolts>();
    let [pending] = bolts.pending_victims().collect::<Vec<_>>()[..] else {
        panic!("the victim should remain visible while the killing bolts are in flight");
    };
    assert_eq!(pending.snapshot, victim);
    assert_position(pending.transform.translation.truncate(), death_position);
    assert_eq!(
        app.world()
            .resource::<ActiveCombatEffects>()
            .death_pops()
            .len(),
        0
    );
    assert_eq!(
        app.world().resource::<ActiveCombatEffects>().decal_count(),
        0
    );

    advance(&mut app, 2);
    assert!(
        app.world()
            .resource::<ActiveCombatBolts>()
            .pending_victims()
            .next()
            .is_none(),
        "the death effects replace the frozen victim on first arrival",
    );
    let effects = app.world().resource::<ActiveCombatEffects>();
    let [pop] = effects.death_pops().collect::<Vec<_>>()[..] else {
        panic!("the first killing bolt should trigger one death pop");
    };
    assert_eq!(pop.victim, victim);
    assert_eq!(pop.color, swarm_presentation_color(OPPONENT));
    assert_eq!(effects.decal_count(), 1);
    assert_eq!(
        effects
            .particles()
            .filter(|particle| particle.kind == CombatParticleKind::Fragment)
            .count(),
        8
    );
    let remaining = app
        .world()
        .resource::<ActiveCombatBolts>()
        .iter()
        .next()
        .unwrap();
    assert_eq!(remaining.attacker, slow_shot.entity);
    assert_position(remaining.end, death_position);

    advance(&mut app, 8);
    assert_eq!(
        app.world().resource::<ActiveCombatEffects>().muzzle_count(),
        2
    );
    advance(&mut app, 2);
    assert_eq!(
        app.world().resource::<ActiveCombatEffects>().muzzle_count(),
        0
    );
    advance(&mut app, 40);
    assert!(app.world().resource::<ActiveCombatBolts>().is_empty());
    let effects = app.world().resource::<ActiveCombatEffects>();
    assert_eq!(effects.death_pops().len(), 1);
    assert_eq!(
        effects.frame_spawns().death_pops,
        0,
        "a later impact cannot replay death"
    );
    assert_eq!(
        effects
            .particles()
            .filter(|particle| particle.kind == CombatParticleKind::ImpactSpark)
            .count(),
        12
    );

    advance(&mut app, 250);
    assert_eq!(
        app.world()
            .resource::<ActiveCombatEffects>()
            .death_pops()
            .len(),
        0
    );
    for _ in 0..2 {
        advance(&mut app, 250);
    }
    let effects = app.world().resource::<ActiveCombatEffects>();
    assert_eq!(effects.particles().len(), 0);
    assert_eq!(
        effects.decal_count(),
        1,
        "the scorch persists after the particles expire"
    );
}

#[test]
fn structure_hit_flashes_at_its_center_and_live_retuning_preserves_progress() {
    let mut app = presentation_app();
    let start = Vec2::new(-32.0, 0.0);
    let target_position = Vec2::new(32.0, 0.0);
    let attacker = common::spawn_defender_at(&mut app, start);
    let target_transform = Transform::from_translation(target_position.extend(GAMEPLAY_SPRITE_Z))
        .with_rotation(Quat::from_rotation_z(0.37))
        .with_scale(Vec3::new(1.25, 0.8, 1.0));
    let neutral_color = completed_visual_color();
    let visual = StructureVisual::completed(PlannedKind::Charger);
    let target = app
        .world_mut()
        .spawn((
            Sprite {
                color: neutral_color,
                ..default()
            },
            target_transform,
            visual,
        ))
        .id();
    advance(&mut app, 10);
    emit_hit(
        &mut app,
        nanobot_snapshot(attacker, start, SwarmId::PLAYER, NanobotType::Defender),
        structure_snapshot(target, target_position - Vec2::Y * 20.0, visual),
        false,
    );
    advance(&mut app, 10);

    let bolt = app
        .world()
        .resource::<ActiveCombatBolts>()
        .iter()
        .next()
        .unwrap();
    assert_position(bolt.end, target_position);
    assert_eq!(
        app.world().get::<Sprite>(target).unwrap().color,
        neutral_color
    );

    advance(&mut app, 41);
    assert!(app.world().resource::<ActiveCombatBolts>().is_empty());
    assert_ne!(
        app.world().get::<Sprite>(target).unwrap().color,
        neutral_color
    );
    assert_eq!(
        app.world().get::<Transform>(target),
        Some(&target_transform)
    );

    advance(&mut app, 90);
    let flash = app.world().get::<Sprite>(target).unwrap().color;
    assert_ne!(flash, neutral_color);
    let old = app.world().resource::<Tuning>().clone();
    let mut next = old.clone();
    next.presentation.combat_recovery_duration_secs = 0.4;
    reconcile_combat_presentation_tuning(app.world_mut(), &old, &next);
    app.insert_resource(next);
    advance(&mut app, 0);
    assert_eq!(app.world().get::<Sprite>(target).unwrap().color, flash);

    advance(&mut app, 190);
    assert_ne!(
        app.world().get::<Sprite>(target).unwrap().color,
        neutral_color
    );
    advance(&mut app, 20);
    assert_eq!(
        app.world().get::<Sprite>(target).unwrap().color,
        neutral_color
    );
    assert_eq!(
        app.world().get::<Transform>(target),
        Some(&target_transform)
    );
}

#[test]
fn a_structure_death_without_a_bolt_plays_immediately_and_refreshes_the_grid_decal() {
    let mut app = presentation_app();
    advance(&mut app, 10);
    let visual = StructureVisual::completed(PlannedKind::ProductionFacility);
    for position in [Vec2::new(5.0, 4.0), Vec2::new(63.0, 62.0)] {
        let victim = app.world_mut().spawn_empty().id();
        emit_death(&mut app, structure_snapshot(victim, position, visual));
        advance(&mut app, 10);

        let effects = app.world().resource::<ActiveCombatEffects>();
        assert_eq!(effects.frame_spawns().death_pops, 1);
        let pop = effects
            .death_pops()
            .find(|pop| pop.victim.entity == victim)
            .unwrap();
        assert_position(pop.victim.position, position);
        assert_eq!(effects.frame_spawns().particles, 16);
        assert_eq!(effects.decal_count(), 1);
    }
    advance(&mut app, 250);
    advance(&mut app, 250);
    let effects = app.world().resource::<ActiveCombatEffects>();
    assert_eq!(effects.particles().len(), 0);
    assert_eq!(effects.decal_count(), 1);
}

#[test]
fn visible_bolts_obey_budget_without_suppressing_defender_aim() {
    let mut app = presentation_app();
    app.world_mut().spawn((
        MainCamera,
        Transform::default(),
        Projection::Orthographic(OrthographicProjection {
            area: Rect::from_corners(Vec2::splat(-250.0), Vec2::splat(250.0)),
            ..OrthographicProjection::default_2d()
        }),
    ));
    advance(&mut app, 10);
    let shot = |app: &mut App, start, end, lethal| {
        let attacker = app.world_mut().spawn_empty().id();
        let target = app.world_mut().spawn_empty().id();
        emit_hit(
            app,
            nanobot_snapshot(attacker, start, SwarmId::PLAYER, NanobotType::Defender),
            nanobot_snapshot(target, end, OPPONENT, NanobotType::Worker),
            lethal,
        );
        attacker
    };

    let mut rejected_shot = |start, end, lethal| {
        let attacker = common::spawn_defender_at(&mut app, start);
        let target = app.world_mut().spawn_empty().id();
        emit_hit(
            &mut app,
            nanobot_snapshot(attacker, start, SwarmId::PLAYER, NanobotType::Defender),
            nanobot_snapshot(target, end, OPPONENT, NanobotType::Worker),
            lethal,
        );
        attacker
    };
    let offscreen = rejected_shot(Vec2::X * 300.0, Vec2::X * 332.0, true);
    let over_budget = rejected_shot(Vec2::new(-32.0, 230.0), Vec2::new(32.0, 230.0), false);
    for _ in 0..300 {
        shot(
            &mut app,
            Vec2::new(-32.0, 120.0),
            Vec2::new(32.0, 120.0),
            false,
        );
    }
    let lethal = shot(
        &mut app,
        Vec2::new(-32.0, 220.0),
        Vec2::new(32.0, 220.0),
        true,
    );
    let near = shot(&mut app, Vec2::X * 300.0, Vec2::ZERO, false);
    advance(&mut app, 10);

    let bolts = app.world().resource::<ActiveCombatBolts>();
    assert_eq!(bolts.len(), 300);
    assert!(!bolts.iter().any(|bolt| bolt.attacker == offscreen));
    assert!(!bolts.iter().any(|bolt| bolt.attacker == over_budget));
    assert!(bolts.iter().any(|bolt| bolt.attacker == lethal));
    assert!(bolts.iter().any(|bolt| bolt.attacker == near));

    advance(&mut app, 200);
    for attacker in [offscreen, over_budget] {
        let pose = app.world().get::<PresentedNanobot>(attacker).unwrap();
        assert_position((pose.rotation() * Vec3::Y).truncate(), Vec2::X);
    }
}

fn injected_reaction(
    attacker_positions: &[Vec2],
    spawn_order: &[usize],
    hit_order: &[usize],
) -> (Vec2, f32) {
    let mut app = presentation_app();
    let position = Vec2::ZERO;
    let target = common::spawn_worker_at(&mut app, position);
    let target_snapshot = nanobot_snapshot(target, position, OPPONENT, NanobotType::Worker);
    let mut attackers = vec![None; attacker_positions.len()];
    for &index in spawn_order {
        let position = attacker_positions[index];
        attackers[index] = Some(nanobot_snapshot(
            common::spawn_defender_at(&mut app, position),
            position,
            SwarmId::PLAYER,
            NanobotType::Defender,
        ));
    }
    advance(&mut app, 10);
    for &index in hit_order {
        emit_hit(&mut app, attackers[index].unwrap(), target_snapshot, false);
    }
    advance(&mut app, 10);
    advance(&mut app, 41);
    assert!(app.world().resource::<ActiveCombatBolts>().is_empty());

    let target_visual = common::nanobot_visual_child(app.world(), target);
    let reaction = app
        .world()
        .get::<Transform>(target_visual)
        .unwrap()
        .translation
        .truncate();
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
fn simultaneous_impacts_combine_into_one_deterministic_bounded_reaction() {
    let positions = [
        Vec2::new(-2.0, -1.0).normalize() * 64.0,
        Vec2::new(-2.0, 1.0).normalize() * 64.0,
        Vec2::NEG_Y * 64.0,
    ];
    let (forward, crowded_flash) = injected_reaction(&positions, &[0, 1, 2], &[0, 1, 2]);
    let (reverse, reverse_flash) = injected_reaction(&positions, &[2, 1, 0], &[2, 1, 0]);
    let (_, single_flash) = injected_reaction(&positions[..1], &[0], &[0]);

    assert_position(forward.normalize(), Vec2::new(0.872_871_6, 0.487_950_03));
    assert_position(forward, reverse);
    assert_abs_diff_eq!(crowded_flash, reverse_flash, epsilon = 0.001);
    assert!(crowded_flash > single_flash);
    let settings = PresentationTuning::default();
    assert!(forward.length() <= settings.combat_recoil_distance + 0.001);
    assert!(crowded_flash <= 1.0 + settings.combat_reaction_flash_cap + 0.001);
}

#[test]
fn simultaneous_impacts_from_opposite_sides_choose_a_stable_recoil() {
    let positions = [Vec2::NEG_X * 64.0, Vec2::X * 64.0];
    let (forward, _) = injected_reaction(&positions, &[0, 1], &[0, 1]);
    let (reverse, _) = injected_reaction(&positions, &[1, 0], &[1, 0]);

    assert!(forward.length() > 0.0);
    assert_position(forward, reverse);
}

#[test]
fn health_arc_tracks_current_damage_band_and_fades_from_the_drawn_position() {
    let mut app = presentation_app();
    let position = Vec2::new(20.0, 30.0);
    let target = common::spawn_worker_at(&mut app, position);
    advance(&mut app, 10);
    let arc = |app: &App| {
        app.world()
            .resource::<ActiveCombatIndicators>()
            .iter()
            .find(|indicator| indicator.owner == target)
            .copied()
    };
    assert!(arc(&app).is_none());

    for (health, band) in [(61, "green"), (60, "amber"), (31, "amber"), (30, "red")] {
        app.world_mut().get_mut::<Health>(target).unwrap().current = health;
        advance(&mut app, 10);
        let indicator = arc(&app).expect("damage should show the target's health arc");
        let CombatIndicatorKind::HealthArc { fraction } = indicator.kind else {
            panic!("a damaged Worker only shows health");
        };
        assert_abs_diff_eq!(fraction, health as f32 / 100.0, epsilon = 1e-5);
        assert_position(indicator.position, position);
        let color = indicator.color.to_srgba();
        assert!(match band {
            "green" => color.green > color.red && color.green > color.blue,
            "amber" => color.red > color.green && color.green > color.blue,
            "red" => color.red > color.green && (color.green - color.blue).abs() < 0.001,
            _ => unreachable!(),
        });
    }

    let new_position = position + Vec2::new(30.0, 40.0);
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation = new_position.extend(0.0);
    for _ in 0..4 {
        advance(&mut app, 250);
    }
    let fading = arc(&app).expect("the health arc should remain visible after one second");
    assert_position(fading.position, new_position);
    assert_abs_diff_eq!(fading.color.alpha(), 0.5, epsilon = 0.01);
    for _ in 0..4 {
        advance(&mut app, 250);
    }
    assert!(arc(&app).is_none());
}

#[test]
fn charge_core_scales_swarm_brightness_and_pulses_when_weakened() {
    let mut app = presentation_app();
    let position = Vec2::new(-20.0, 40.0);
    let defender = common::spawn_defender_at(&mut app, position);
    app.world_mut().entity_mut(defender).insert((
        SwarmMember::new(OPPONENT),
        Charge {
            current: 0.8,
            max: 1.0,
        },
    ));
    advance(&mut app, 10);

    let brightness = |app: &App| {
        let core = app
            .world()
            .resource::<ActiveCombatIndicators>()
            .iter()
            .find(|indicator| {
                indicator.owner == defender && indicator.kind == CombatIndicatorKind::ChargeCore
            })
            .expect("a Defender should have a charge core");
        assert_position(core.position, position);
        let actual = core.color.to_linear();
        let faction = swarm_presentation_color(OPPONENT).to_linear();
        let strength = actual.red / faction.red;
        assert_abs_diff_eq!(actual.green, strength * faction.green, epsilon = 1e-5);
        assert_abs_diff_eq!(actual.blue, strength * faction.blue, epsilon = 1e-5);
        strength
    };
    assert_abs_diff_eq!(brightness(&app), 0.8, epsilon = 1e-5);

    app.world_mut().get_mut::<Charge>(defender).unwrap().current = 0.1;
    advance(&mut app, 167);
    let first = brightness(&app);
    advance(&mut app, 167);
    advance(&mut app, 166);
    let half_cycle = brightness(&app);
    assert!((0.299..=0.701).contains(&first));
    assert!((0.299..=0.701).contains(&half_cycle));
    assert!((first - half_cycle).abs() > 0.35);
}

#[test]
fn reduced_effects_keep_sparks_and_smoke_for_structures_below_half_health() {
    let mut app = presentation_app();
    let half_health = app
        .world()
        .resource::<Tuning>()
        .construction
        .max_health
        .div_ceil(2);
    app.world_mut()
        .resource_mut::<Tuning>()
        .presentation
        .effects_intensity = 0.3;
    let owner = app.world_mut().spawn((Swarm {}, OPPONENT)).id();
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        owner,
        PlannedKind::Charger,
        Transform::default(),
        Some(half_health),
    );

    for _ in 0..12 {
        advance(&mut app, 100);
        assert_eq!(
            app.world()
                .resource::<ActiveCombatEffects>()
                .particles()
                .len(),
            0
        );
    }

    app.world_mut()
        .get_mut::<Structure>(structure)
        .unwrap()
        .health -= 1;
    let mut saw_spark = false;
    let mut saw_smoke = false;
    for _ in 0..40 {
        advance(&mut app, 100);
        for particle in app.world().resource::<ActiveCombatEffects>().particles() {
            saw_spark |= particle.kind == CombatParticleKind::StructureSpark;
            saw_smoke |= particle.kind == CombatParticleKind::Smoke;
        }
        if saw_spark && saw_smoke {
            break;
        }
    }
    assert!(
        saw_spark && saw_smoke,
        "reduced effects should retain both damage cues"
    );
}
