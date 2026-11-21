use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use bevy::{asset::AssetPlugin, prelude::*, time::TimeUpdateStrategy};
use top_down_2d_rts_prototype_nano_swarm::{
    fixed_simulation_time,
    nanobot::{
        ActiveCombatEffects, Cargo, CombatAppearance, CombatVisualSnapshot, Health,
        MaintenanceProgress, NanobotPresentationPlugin, NanobotType, ResolvedCombatDeath,
        ResolvedCombatFact, ResolvedCombatHit, SwarmId, VelocityComponent,
        nanobot_death_cleanup_system, rotation_for_direction,
    },
    resources::{ResourceKind, ResourceLedger},
    structure_overlay::{
        ConditionOverlay, ConditionOverlayKind, StructureOverlay, StructureOverlayPlugin,
    },
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/combat_scenes.rs"]
mod scenes;

const FRAME_DELTA: Duration = Duration::from_nanos(6_944_444);

fn add_presentation(app: &mut App) {
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins((TransformPlugin, NanobotPresentationPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_DELTA));
}

fn visual_transform(app: &App, root: Entity) -> &GlobalTransform {
    let visual = common::nanobot_visual_child(app.world(), root);
    app.world().get::<GlobalTransform>(visual).unwrap()
}

fn visual_heading(app: &App, root: Entity) -> Vec2 {
    visual_transform(app, root).up().truncate()
}

fn angle_between(from: Vec2, to: Vec2) -> f32 {
    from.angle_to(to).abs().to_degrees()
}

fn motion_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .insert_resource(fixed_simulation_time())
        .add_systems(FixedUpdate, advance_scripted_motion);
    add_presentation(&mut app);
    app
}

fn advance_scripted_motion(
    time: Res<Time<Fixed>>,
    mut bodies: Query<(&VelocityComponent, &mut Transform)>,
) {
    for (velocity, mut root) in &mut bodies {
        root.translation += velocity.value.extend(0.0) * time.delta_secs();
        if let Some(rotation) = rotation_for_direction(velocity.value) {
            root.rotation = rotation;
        }
    }
}

fn set_motion(app: &mut App, root: Entity, velocity: Vec2) {
    app.world_mut()
        .get_mut::<VelocityComponent>(root)
        .unwrap()
        .value = velocity;
}

#[test]
fn teleported_nanobot_draws_at_its_destination_immediately() {
    let mut app = motion_app();
    let defender = common::spawn_defender_at(&mut app, Vec2::new(100.0, 100.0));
    set_motion(&mut app, defender, Vec2::X * 180.0);
    for _ in 0..8 {
        app.update();
    }

    let before_jump = app.world().get::<Transform>(defender).unwrap().translation;
    set_motion(&mut app, defender, Vec2::X * 1_800.0);
    for _ in 0..3 {
        app.update();
        let root = app.world().get::<Transform>(defender).unwrap().translation;
        if root.x > before_jump.x + 25.0 {
            let drawn = visual_transform(&app, defender).translation();
            assert!(
                drawn.distance(root) <= 0.01,
                "a teleport from {before_jump} to {root} drew at {drawn}",
            );
            return;
        }
    }
    panic!("fixture must perform the jump on its next fixed tick");
}

#[derive(Component)]
struct LethalHit;

#[test]
fn moving_nanobot_death_pop_starts_at_the_interpolated_position() {
    let mut app = motion_app();
    app.init_resource::<ResourceLedger>()
        .add_systems(
            FixedUpdate,
            (
                |mut target: Single<(Entity, &Transform, &mut Health), With<LethalHit>>,
                 mut facts: MessageWriter<ResolvedCombatFact>| {
                    target.2.current = 0;
                    facts.write(ResolvedCombatFact::Death(ResolvedCombatDeath {
                        victim: CombatVisualSnapshot {
                            entity: target.0,
                            position: target.1.translation.truncate(),
                            appearance: CombatAppearance::Nanobot(NanobotType::Worker),
                            swarm: SwarmId::PLAYER,
                        },
                    }));
                },
            )
                .after(advance_scripted_motion),
        )
        .add_systems(FixedLast, nanobot_death_cleanup_system);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 100.0));
    set_motion(&mut app, worker, Vec2::X * 180.0);
    for _ in 0..8 {
        app.update();
    }
    let last_fixed = app
        .world()
        .get::<Transform>(worker)
        .unwrap()
        .translation
        .truncate();
    app.world_mut().entity_mut(worker).insert(LethalHit);
    for _ in 0..3 {
        app.update();
        let effects = app.world().resource::<ActiveCombatEffects>();
        let Some(pop) = effects.death_pops().next() else {
            continue;
        };
        let fraction = app.world().resource::<Time<Fixed>>().overstep_fraction();
        let expected = last_fixed + Vec2::X * (3.0 * fraction);
        assert!(app.world().get_entity(worker).is_err());
        assert!(
            pop.victim.position.distance(expected) <= 0.01,
            "death pop started at {:?}, expected {expected}",
            pop.victim.position,
        );
        return;
    }
    panic!("the destroyed Worker should produce a death pop");
}

#[test]
fn reversing_defender_turns_within_the_frame_turn_budget() {
    let mut app = motion_app();
    let defender = common::spawn_defender_at(&mut app, Vec2::new(100.0, 100.0));
    set_motion(&mut app, defender, Vec2::X * 180.0);
    for _ in 0..48 {
        app.update();
    }
    let mut previous = visual_heading(&app, defender);
    assert!(angle_between(previous, Vec2::X) <= 0.5);

    set_motion(&mut app, defender, Vec2::NEG_X * 180.0);
    let turn_rate = app
        .world()
        .resource::<Tuning>()
        .presentation
        .defender_turn_rate;
    let budget = turn_rate * FRAME_DELTA.as_secs_f32() + 0.5;
    let mut max_turn: f32 = 0.0;
    for frame in 0..48 {
        app.update();
        let heading = visual_heading(&app, defender);
        let turned = angle_between(previous, heading);
        assert!(
            turned <= budget,
            "frame {frame} turned {turned}°, budget {budget}°"
        );
        max_turn = max_turn.max(turned);
        previous = heading;
    }
    assert!(angle_between(previous, Vec2::NEG_X) <= 0.5);
    println!("144 Hz Defender reversal: max frame turn {max_turn:.3}°");
}

#[test]
fn cargo_and_progress_bars_follow_presented_bodies_every_frame() {
    let mut app = motion_app();
    app.add_plugins(StructureOverlayPlugin);
    let worker = common::spawn_worker_at(&mut app, Vec2::new(100.0, 100.0));
    let hauler = common::spawn_hauler_at(&mut app, Vec2::new(100.0, 200.0));
    let work_site = app.world_mut().spawn_empty().id();
    for root in [worker, hauler] {
        app.world_mut().entity_mut(root).insert(Cargo {
            kind: ResourceKind::Minerals,
            amount: 2,
        });
        set_motion(&mut app, root, Vec2::X * 180.0);
    }
    app.world_mut()
        .entity_mut(worker)
        .insert(MaintenanceProgress {
            cell: IVec2::ZERO,
            target: work_site,
            ticks_worked: 1,
        });
    for _ in 0..8 {
        app.update();
    }
    let world = app.world_mut();
    let bars: Vec<_> = world
        .query::<(&StructureOverlay, Entity)>()
        .iter(world)
        .map(|(bar, entity)| (bar.target, entity))
        .collect();
    assert_eq!(bars.len(), 2);
    assert!(bars.iter().any(|&(target, _)| target == worker));
    assert!(bars.iter().any(|&(target, _)| target == hauler));
    let (progress, progress_bar) = world
        .query::<(&ConditionOverlay, Entity)>()
        .single(world)
        .unwrap();
    assert_eq!(progress.kind, ConditionOverlayKind::WorkerProgress);
    assert_eq!(progress.target, worker);

    let offset = |app: &App, root, bar| {
        let bar_position = app
            .world()
            .get::<GlobalTransform>(bar)
            .unwrap()
            .translation();
        (bar_position - visual_transform(app, root).translation()).truncate()
    };
    let mut followed: Vec<_> = bars
        .into_iter()
        .chain([(worker, progress_bar)])
        .map(|(root, bar)| {
            let initial = offset(&app, root, bar);
            assert!(
                initial.y > 0.0,
                "following bar should appear above its body"
            );
            (root, bar, initial, initial)
        })
        .collect();
    let attacker = app.world_mut().spawn_empty().id();
    let mut max_drift: f32 = 0.0;
    let mut max_recoil = [0.0_f32; 2];
    for frame in 0..48 {
        if frame == 8 {
            for (target, kind) in [(worker, NanobotType::Worker), (hauler, NanobotType::Hauler)] {
                let position = app
                    .world()
                    .get::<Transform>(target)
                    .unwrap()
                    .translation
                    .truncate();
                app.world_mut()
                    .write_message(ResolvedCombatFact::Hit(ResolvedCombatHit {
                        attacker: CombatVisualSnapshot {
                            entity: attacker,
                            position: position - Vec2::X * 90.0,
                            appearance: CombatAppearance::Nanobot(NanobotType::Defender),
                            swarm: SwarmId(1),
                        },
                        target: CombatVisualSnapshot {
                            entity: target,
                            position,
                            appearance: CombatAppearance::Nanobot(kind),
                            swarm: SwarmId::PLAYER,
                        },
                        damage: 10,
                        target_destroyed: false,
                    }));
            }
        }
        app.update();
        for (index, root) in [worker, hauler].into_iter().enumerate() {
            let simulation_x = app.world().get::<Transform>(root).unwrap().translation.x;
            let recoil = visual_transform(&app, root).translation().x - simulation_x;
            max_recoil[index] = max_recoil[index].max(recoil);
        }
        for (root, bar, initial, previous) in &mut followed {
            let current = offset(&app, *root, *bar);
            let drift = current.distance(*initial).max(current.distance(*previous));
            assert!(
                drift <= 0.5 && current.x.abs() <= 0.5,
                "bar {bar} drifted from {initial:?} to {current:?} on frame {frame}",
            );
            max_drift = max_drift.max(drift);
            *previous = current;
        }
    }
    assert!(max_recoil.iter().all(|&recoil| recoil > 5.0));
    println!("144 Hz cargo and progress bars: max deviation {max_drift:.4} units");
}

#[test]
fn skirmish_defenders_hold_aim_on_their_latest_hit() {
    let mut app = scenes::scene_app();
    add_presentation(&mut app);
    scenes::spawn_scene(
        app.world_mut(),
        scenes::Scene::Skirmish,
        scenes::Variant::Standard,
    );
    let mut facts = app
        .world()
        .resource::<Messages<ResolvedCombatFact>>()
        .get_cursor();
    let turn_rate = app
        .world()
        .resource::<Tuning>()
        .presentation
        .defender_turn_rate;
    let mut aiming = HashMap::new();
    let mut attackers = HashSet::new();
    let mut checked = HashSet::new();
    let mut max_aim_error: f32 = 0.0;

    for _ in 0..(12 * 144) {
        app.update();
        let tick_time = app.world().resource::<Time<Fixed>>().elapsed();
        let frame_time = app.world().resource::<Time<Virtual>>().elapsed();
        for fact in facts.read(app.world().resource::<Messages<ResolvedCombatFact>>()) {
            if let ResolvedCombatFact::Hit(hit) = fact {
                let attacker = hit.attacker.entity;
                let direction = (hit.target.position - hit.attacker.position).normalize();
                aiming.insert(attacker, (tick_time, direction));
                attackers.insert(attacker);
            }
        }
        for (&entity, &(latest_hit, direction)) in &aiming {
            let elapsed = frame_time.saturating_sub(latest_hit).as_secs_f32();
            if elapsed >= 180.0 / turn_rate
                && elapsed < 0.6
                && app.world().get_entity(entity).is_ok()
            {
                let error = angle_between(visual_heading(&app, entity), direction);
                assert!(
                    error <= 10.0,
                    "Defender {entity} aimed {error}° away from its latest target {elapsed:.3}s after hitting"
                );
                max_aim_error = max_aim_error.max(error);
                checked.insert(entity);
            }
        }
    }
    assert!(
        attackers.len() >= 2,
        "the skirmish should exercise both sides' aim"
    );
    assert_eq!(checked, attackers);
    println!(
        "144 Hz skirmish: max settled aim error {max_aim_error:.3}° across {} Defenders",
        checked.len()
    );
}
