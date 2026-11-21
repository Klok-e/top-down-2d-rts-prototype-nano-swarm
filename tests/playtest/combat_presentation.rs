use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    MainCamera,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActiveCombatBolts, ActiveCombatEffects, Charge, CombatBolt, CombatDeathPop,
        CombatParticleKind, DefenderAttackCooldown, DefenderResponse, Health,
        NanobotPresentationPlugin, NanobotSimulationSet, NonCombatDestruction, OpponentSwarm,
        PlannedKind, Structure, Swarm, SwarmId, SwarmMember, completed_visual_color,
        defender_health_loss_when_empty_system, nanobot_death_cleanup_system,
        structure_degradation_system, swarm_presentation_color, world_to_cell,
    },
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

const OPPONENT: SwarmId = SwarmId(19);
const TIMING_MARGIN: Duration = Duration::from_millis(1);

fn combat_presentation_app() -> (App, Entity) {
    let mut app = common::sim_app_with_combat();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
        .init_asset::<Image>()
        .add_plugins((TransformPlugin, NanobotPresentationPlugin))
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    app.world_mut().resource_mut::<Tuning>().logistics.bot_speed = 0.0;
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    let opponent = app
        .world_mut()
        .spawn((Swarm {}, OPPONENT, OpponentSwarm {}))
        .id();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    (app, opponent)
}

fn advance_frame(app: &mut App, duration: Duration) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(duration));
    app.update();
}

fn bolts(app: &App) -> &ActiveCombatBolts {
    app.world().resource::<ActiveCombatBolts>()
}

fn only_bolt(app: &App) -> &CombatBolt {
    assert_eq!(bolts(app).len(), 1);
    bolts(app).iter().next().unwrap()
}

fn effects(app: &App) -> &ActiveCombatEffects {
    app.world().resource::<ActiveCombatEffects>()
}

fn only_death_pop(app: &App) -> &CombatDeathPop {
    let mut pops = effects(app).death_pops();
    assert_eq!(pops.len(), 1);
    pops.next().unwrap()
}

fn remaining_flight(bolt: &CombatBolt) -> Duration {
    bolt.flight_time - bolt.elapsed
}

fn advance_through_arrivals(app: &mut App) {
    let last_arrival = bolts(app)
        .iter()
        .map(remaining_flight)
        .max()
        .expect("at least one bolt should be in flight");
    advance_frame(app, last_arrival + TIMING_MARGIN);
    assert!(bolts(app).is_empty());
}

fn tint(app: &App, visual: Entity) -> Color {
    app.world().get::<Sprite>(visual).unwrap().color
}

fn visual_position(app: &App, visual: Entity) -> Vec2 {
    app.world()
        .get::<GlobalTransform>(visual)
        .unwrap()
        .translation()
        .truncate()
}

#[test]
fn real_hit_bolt_tracks_presented_target_and_flashes_at_arrival() {
    let (mut app, _) = combat_presentation_app();
    let center = common::cell_world_center(IVec2::ZERO);
    let attacker = common::spawn_defender_at(&mut app, center - Vec2::X * 36.0);
    let target = common::spawn_worker_at(&mut app, center + Vec2::X * 36.0);
    app.world_mut()
        .entity_mut(target)
        .insert(SwarmMember::new(OPPONENT));
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));
    let initial_health = app.world().get::<Health>(target).unwrap().current;
    let attacker_root = *app.world().get::<Transform>(attacker).unwrap();

    app.update();

    assert!(app.world().get::<Health>(target).unwrap().current < initial_health);
    let attacker_visual = common::nanobot_visual_child(app.world(), attacker);
    let target_visual = common::nanobot_visual_child(app.world(), target);
    let bolt = only_bolt(&app);
    assert_eq!((bolt.attacker, bolt.target), (attacker, target));
    let launch_position = visual_position(&app, attacker_visual);
    let target_position = visual_position(&app, target_visual);
    let expected_seconds = (launch_position.distance(target_position) / 1600.0).clamp(0.04, 0.09);
    assert!(launch_position.distance(attacker_root.translation.truncate()) > 0.01);
    assert!((bolt.flight_time.as_secs_f32() - expected_seconds).abs() <= 0.000_001);
    assert!(bolt.start.abs_diff_eq(launch_position, 0.01));
    assert_eq!(tint(&app, target_visual), Color::WHITE);
    assert!(
        effects(&app)
            .particles()
            .all(|particle| particle.kind != CombatParticleKind::ImpactSpark)
    );

    let until_just_before_arrival = remaining_flight(bolt) - TIMING_MARGIN;
    let mut relocated_target = *app.world().get::<Transform>(target).unwrap();
    relocated_target.translation.y += 48.0;
    *app.world_mut().get_mut::<Transform>(target).unwrap() = relocated_target;
    advance_frame(&mut app, until_just_before_arrival);

    let bolt = only_bolt(&app);
    let presented_target = visual_position(&app, target_visual);
    assert!(bolt.end.abs_diff_eq(presented_target, 0.01));
    assert!(bolt.position.distance(presented_target) < 3.0);
    assert_eq!(tint(&app, target_visual), Color::WHITE);

    advance_through_arrivals(&mut app);

    assert_ne!(tint(&app, target_visual), Color::WHITE);
    assert!(
        app.world()
            .get::<Transform>(target_visual)
            .unwrap()
            .translation
            .length()
            > 0.0,
        "arrival should start the target recoil",
    );
    assert_eq!(
        effects(&app)
            .particles()
            .filter(|particle| particle.kind == CombatParticleKind::ImpactSpark)
            .count(),
        6,
        "the impact should start one normal-intensity spark burst",
    );

    let recovery = Duration::from_secs_f32(
        app.world()
            .resource::<Tuning>()
            .presentation
            .combat_recovery_duration_secs,
    );
    advance_frame(&mut app, recovery + Duration::from_millis(5));
    for visual in [attacker_visual, target_visual] {
        let pose = app.world().get::<Transform>(visual).unwrap();
        assert!(pose.translation.abs_diff_eq(Vec3::ZERO, 0.001));
        assert!(pose.scale.abs_diff_eq(Vec3::ONE, 0.001));
    }
    assert_eq!(tint(&app, target_visual), Color::WHITE);
    assert!(
        effects(&app)
            .particles()
            .all(|particle| particle.kind != CombatParticleKind::ImpactSpark)
    );
    assert_eq!(app.world().get::<Transform>(attacker), Some(&attacker_root));
    assert_eq!(
        app.world().get::<Transform>(target),
        Some(&relocated_target)
    );
}

#[test]
fn real_nanobot_death_waits_for_first_killing_bolt_and_plays_once() {
    let (mut app, _) = combat_presentation_app();
    let cell = IVec2::ZERO;
    let center = common::cell_world_center(cell);
    let near = common::spawn_defender_at(&mut app, center - Vec2::X * 72.0);
    let far = common::spawn_defender_at(&mut app, center + Vec2::X * 108.0);
    let target = common::spawn_worker_at(&mut app, center);
    app.world_mut().entity_mut(target).insert((
        SwarmMember::new(OPPONENT),
        Health {
            current: 1,
            max: 100,
        },
    ));
    for attacker in [near, far] {
        app.world_mut()
            .entity_mut(attacker)
            .insert(DefenderAttackCooldown::new(1));
    }
    app.world_mut()
        .entity_mut(near)
        .insert(DefenderResponse { target });
    let attacker_root = *app.world().get::<Transform>(near).unwrap();

    app.update();
    let target_visual = common::nanobot_visual_child(app.world(), target);
    let target_image = app
        .world()
        .get::<Sprite>(target_visual)
        .unwrap()
        .image
        .clone();
    let drawn_target = visual_position(&app, target_visual);

    app.update();

    assert!(!app.world().entities().contains(target));
    assert_eq!(bolts(&app).len(), 2);
    assert_eq!(effects(&app).death_pops().len(), 0);
    let pending = bolts(&app).pending_victims().next().unwrap();
    assert_eq!(pending.snapshot.entity, target);
    assert_eq!(pending.sprite.as_ref().unwrap().image, target_image);
    assert!(
        pending
            .transform
            .translation
            .truncate()
            .abs_diff_eq(drawn_target, 0.01)
    );
    let first_arrival = bolts(&app).iter().map(remaining_flight).min().unwrap();
    advance_frame(&mut app, first_arrival - TIMING_MARGIN);
    assert_eq!(bolts(&app).len(), 2);
    assert_eq!(effects(&app).death_pops().len(), 0);

    advance_frame(&mut app, 2 * TIMING_MARGIN);

    assert_eq!(only_bolt(&app).attacker, far);
    assert!(only_bolt(&app).end.abs_diff_eq(center, 0.01));
    assert!(bolts(&app).pending_victims().next().is_none());
    let pop = only_death_pop(&app);
    assert_eq!(pop.victim.entity, target);
    assert!(pop.victim.position.abs_diff_eq(center, 0.01));
    let first_pop_age = pop.elapsed_secs();

    advance_through_arrivals(&mut app);

    assert!(only_death_pop(&app).elapsed_secs() > first_pop_age);
    assert_eq!(effects(&app).decal_count(), 1);
    assert_eq!(app.world().get::<Transform>(near), Some(&attacker_root));

    app.world_mut().resource_mut::<Tuning>().logistics.bot_speed = 5.0;
    for _ in 0..4 {
        advance_frame(&mut app, Duration::from_millis(200));
    }

    assert_eq!(effects(&app).death_pops().len(), 0);
    assert_eq!(effects(&app).particles().len(), 0);
    assert_eq!(effects(&app).decal_count(), 1);
    assert!(app.world().get::<DefenderResponse>(near).is_none());
    let returned_root = app.world().get::<Transform>(near).unwrap();
    assert_eq!(world_to_cell(returned_root.translation.truncate()), cell);
    assert_ne!(
        returned_root, &attacker_root,
        "the Defender should resume staging"
    );
}

#[test]
fn simultaneous_real_hits_use_both_swarm_colors_and_bound_target_recoil() {
    for (attacker_swarm, target_swarm) in [(SwarmId::PLAYER, OPPONENT), (OPPONENT, SwarmId::PLAYER)]
    {
        let (mut app, _) = combat_presentation_app();
        app.world_mut().resource_mut::<Tuning>().combat.base_attack = 20.0;
        let center = common::cell_world_center(IVec2::ZERO);
        let target = common::spawn_worker_at(&mut app, center);
        app.world_mut()
            .entity_mut(target)
            .insert(SwarmMember::new(target_swarm));
        let attackers = [-Vec2::X, Vec2::X, -Vec2::Y].map(|direction| {
            let attacker = common::spawn_defender_at(&mut app, center + direction * 80.0);
            app.world_mut().entity_mut(attacker).insert((
                SwarmMember::new(attacker_swarm),
                DefenderAttackCooldown::new(0),
            ));
            attacker
        });
        let target_root = *app.world().get::<Transform>(target).unwrap();

        app.update();

        let fired = bolts(&app);
        assert_eq!(fired.len(), attackers.len());
        for attacker in attackers {
            assert!(fired.iter().any(|bolt| {
                bolt.attacker == attacker
                    && bolt.target == target
                    && bolt.color == swarm_presentation_color(attacker_swarm)
            }));
            let visual = common::nanobot_visual_child(app.world(), attacker);
            let root = app.world().get::<Transform>(attacker).unwrap();
            assert!(visual_position(&app, visual).distance(root.translation.truncate()) > 0.01);
        }
        let target_visual = common::nanobot_visual_child(app.world(), target);
        assert_eq!(tint(&app, target_visual), Color::WHITE);

        advance_through_arrivals(&mut app);

        let reaction = app
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
        assert!(reaction.length() > 0.0 && reaction.length() <= recoil_limit + 0.001);
        assert_ne!(tint(&app, target_visual), Color::WHITE);
        assert_eq!(app.world().get::<Transform>(target), Some(&target_root));
    }
}

#[test]
fn real_support_structure_flashes_at_bolt_arrival_and_recovers_in_place() {
    let (mut app, opponent) = combat_presentation_app();
    let center = common::cell_world_center(IVec2::ZERO);
    let attacker = common::spawn_defender_at(&mut app, center - Vec2::X * 36.0);
    let structure_transform = Transform::from_translation((center + Vec2::X * 36.0).extend(1.0))
        .with_rotation(Quat::from_rotation_z(0.31));
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        opponent,
        PlannedKind::Charger,
        structure_transform,
        None,
    );
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));
    let before_health = app.world().get::<Structure>(structure).unwrap().health;

    app.update();

    assert!(app.world().get::<Structure>(structure).unwrap().health < before_health);
    let bolt = only_bolt(&app);
    assert_eq!(bolt.target, structure);
    assert!(
        bolt.end
            .abs_diff_eq(structure_transform.translation.truncate(), 0.01)
    );
    assert_eq!(tint(&app, structure), completed_visual_color());

    advance_through_arrivals(&mut app);

    assert_ne!(tint(&app, structure), completed_visual_color());
    assert_eq!(
        app.world().get::<Transform>(structure),
        Some(&structure_transform)
    );
    let recovery = Duration::from_secs_f32(
        app.world()
            .resource::<Tuning>()
            .presentation
            .combat_recovery_duration_secs,
    );
    advance_frame(&mut app, recovery + Duration::from_millis(5));

    assert_eq!(tint(&app, structure), completed_visual_color());
    assert_eq!(
        app.world().get::<Transform>(structure),
        Some(&structure_transform)
    );
}

#[test]
fn lethal_real_structure_hit_starts_death_effects_on_arrival() {
    let (mut app, opponent) = combat_presentation_app();
    let center = common::cell_world_center(IVec2::ZERO);
    let attacker = common::spawn_defender_at(&mut app, center - Vec2::X * 36.0);
    let death_position = center + Vec2::X * 36.0;
    let structure_transform = Transform::from_translation(death_position.extend(1.0))
        .with_rotation(Quat::from_rotation_z(-0.43));
    let structure = common::spawn_owned_completed_structure(
        &mut app,
        opponent,
        PlannedKind::ProductionFacility,
        structure_transform,
        Some(1),
    );
    let structure_size = app.world().get::<Sprite>(structure).unwrap().custom_size;
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));

    app.update();

    assert!(!app.world().entities().contains(structure));
    assert!(only_bolt(&app).target_destroyed);
    assert_eq!(effects(&app).death_pops().len(), 0);
    let pending = bolts(&app).pending_victims().next().unwrap();
    assert_eq!(pending.snapshot.entity, structure);
    assert_eq!(pending.transform, structure_transform);
    assert_eq!(pending.sprite.as_ref().unwrap().custom_size, structure_size);

    advance_through_arrivals(&mut app);

    assert!(bolts(&app).pending_victims().next().is_none());
    let pop = only_death_pop(&app);
    assert_eq!(pop.victim.entity, structure);
    assert!(pop.victim.position.abs_diff_eq(death_position, 0.01));
    assert_eq!(
        effects(&app)
            .particles()
            .filter(|particle| particle.kind == CombatParticleKind::Fragment)
            .count(),
        16,
    );
    assert_eq!(effects(&app).decal_count(), 1);

    for _ in 0..3 {
        advance_frame(&mut app, Duration::from_millis(200));
    }
    assert_eq!(effects(&app).death_pops().len(), 0);
    assert_eq!(effects(&app).particles().len(), 0);
    assert_eq!(effects(&app).decal_count(), 1);
}

#[test]
fn far_casualties_include_depletion_and_lethal_combat_once_each() {
    let (mut app, _) = combat_presentation_app();
    app.add_systems(
        FixedUpdate,
        defender_health_loss_when_empty_system.before(NanobotSimulationSet::Combat),
    );
    let center = common::cell_world_center(IVec2::ZERO);
    let destroyed_by_combat = center + Vec2::X * 72.0;
    let destroyed_by_depletion = center + Vec2::Y * 512.0;
    app.world_mut().spawn((
        MainCamera,
        Transform::from_translation(center.extend(0.0)),
        Projection::Orthographic(OrthographicProjection {
            scale: 12.0,
            area: Rect::from_center_size(Vec2::ZERO, Vec2::splat(2048.0)),
            ..OrthographicProjection::default_2d()
        }),
    ));
    app.world_mut()
        .resource_mut::<Tuning>()
        .charge
        .empty_damage_interval_ticks = 1;
    let out_of_charge = common::spawn_defender_at(&mut app, destroyed_by_depletion);
    app.world_mut().entity_mut(out_of_charge).insert((
        Charge {
            current: 0.0,
            ..default()
        },
        Health {
            current: 1,
            ..default()
        },
    ));
    let attacker = common::spawn_defender_at(&mut app, center);
    app.world_mut()
        .entity_mut(attacker)
        .insert(DefenderAttackCooldown::new(0));
    let combat_target = common::spawn_worker_at(&mut app, destroyed_by_combat);
    app.world_mut().entity_mut(combat_target).insert((
        Health {
            current: 1,
            ..default()
        },
        SwarmMember::new(OPPONENT),
    ));

    app.update();

    assert!(!app.world().entities().contains(out_of_charge));
    assert!(!app.world().entities().contains(combat_target));
    let effects = effects(&app);
    let flashes = effects.casualty_flashes();
    assert_eq!(flashes.len(), 2);
    assert!(flashes.iter().any(|flash| {
        flash.position.abs_diff_eq(destroyed_by_depletion, 0.01)
            && flash.color == swarm_presentation_color(SwarmId::PLAYER)
    }));
    assert!(flashes.iter().any(|flash| {
        flash.position.abs_diff_eq(destroyed_by_combat, 0.01)
            && flash.color == swarm_presentation_color(OPPONENT)
    }));
    assert!(bolts(&app).is_empty());
    assert_eq!(effects.death_pops().len(), 0);
    assert_eq!(effects.decal_count(), 0);
}

#[test]
fn only_degradation_collapse_reports_noncombat_structure_destruction() {
    let (mut app, opponent) = combat_presentation_app();
    app.add_systems(
        FixedUpdate,
        structure_degradation_system.in_set(NanobotSimulationSet::Maintenance),
    );
    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.maintenance.buffer_ticks = 0;
    tuning.maintenance.degradation_interval_ticks = 1;
    tuning.maintenance.degradation_per_step = 1;

    let position = common::cell_world_center(IVec2::ZERO);
    let collapsing = common::spawn_owned_completed_structure(
        &mut app,
        opponent,
        PlannedKind::ProductionFacility,
        Transform::from_translation(position.extend(0.0)),
        Some(1),
    );
    let removed_during_cleanup = common::spawn_owned_completed_structure(
        &mut app,
        opponent,
        PlannedKind::Charger,
        Transform::from_translation((position + Vec2::Y * 192.0).extend(0.0)),
        None,
    );
    let cancelled_plan = common::spawn_planned_structure_at_cell(&mut app, IVec2::X);
    let mut reader = app
        .world()
        .resource::<Messages<NonCombatDestruction>>()
        .get_cursor_current();
    assert!(app.world_mut().despawn(removed_during_cleanup));
    assert!(app.world_mut().despawn(cancelled_plan));

    app.update();

    assert!(!app.world().entities().contains(collapsing));
    let events = app.world().resource::<Messages<NonCombatDestruction>>();
    let [NonCombatDestruction::Structure { position: reported }] =
        reader.read(events).copied().collect::<Vec<_>>()[..]
    else {
        panic!("only the actual collapse should produce a destruction notification");
    };
    assert_eq!(reported, position);
    assert_eq!(effects(&app).death_pops().len(), 0);
}
