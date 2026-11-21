//! Close-up offscreen evidence for bolt flight, impacts and death effects.

use std::time::Duration;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z, ZONE_BLOCK_SIZE, fixed_simulation_time,
    fly_camera::CameraZoom2d,
    nanobot::{
        ActiveCombatBolts, ActiveCombatEffects, Charge, Commitment, DefenderAttackCooldown, Health,
        Nanobot, NanobotType, NanobotVisual, OpponentSwarm, OwnerSwarm,
        PLANNED_STRUCTURE_FOOTPRINT, PlannedKind, Structure, StructureKind, SwarmId, SwarmMember,
        completed_visual_color,
    },
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

#[derive(Clone, Copy)]
enum Target {
    Defender,
    Worker,
    Facility,
}

impl Target {
    fn lethal(self) -> bool {
        !matches!(self, Self::Defender)
    }

    fn capture_prefix(self) -> &'static str {
        match self {
            Self::Defender => "combat_presentation",
            Self::Worker => "nanobot_combat_death",
            Self::Facility => "support_structure_combat",
        }
    }
}

#[derive(Clone, Copy, Resource)]
struct CombatEvidence {
    attacker: Entity,
    attacker_root: Transform,
    target: Entity,
    target_root: Transform,
    phase: Phase,
}

#[derive(Clone, Copy)]
enum Phase {
    Ready,
    Flight,
    FlightCaptured,
    Arrival,
    Aftermath,
    Exit,
}

pub fn combat_presentation(ctx: &mut TestContext) -> TestFlow {
    capture_combat(ctx, Target::Defender)
}

pub fn nanobot_combat_death(ctx: &mut TestContext) -> TestFlow {
    capture_combat(ctx, Target::Worker)
}

pub fn support_structure_combat_presentation(ctx: &mut TestContext) -> TestFlow {
    capture_combat(ctx, Target::Facility)
}

fn capture_combat(ctx: &mut TestContext, target: Target) -> TestFlow {
    let world = &mut *ctx.world;
    if ctx.frame == 0 {
        setup_scene(world, target);
        return TestFlow::Continue;
    }
    let evidence = *world.resource::<CombatEvidence>();
    assert_eq!(
        world.get::<Transform>(evidence.attacker),
        Some(&evidence.attacker_root)
    );

    match evidence.phase {
        Phase::Ready => {
            world
                .entity_mut(evidence.attacker)
                .insert(DefenderAttackCooldown::new(0));
            world.resource_mut::<Time<Virtual>>().unpause();
            world.resource_mut::<CombatEvidence>().phase = Phase::Flight;
        }
        Phase::Flight => {
            let bolts = world.resource::<ActiveCombatBolts>();
            let Some(bolt) = bolts.iter().find(|bolt| bolt.elapsed > Duration::ZERO) else {
                assert!(
                    ctx.frame < 12,
                    "the Defender should fire at the nearby target"
                );
                return TestFlow::Continue;
            };
            assert_eq!(bolt.attacker, evidence.attacker);
            assert_eq!(bolt.target, evidence.target);
            assert_eq!(bolt.target_destroyed, target.lethal());
            assert!(bolt.position.distance(bolt.start) > 1.0);
            assert!(bolt.position.distance(bolt.end) > 1.0);
            assert_eq!(
                world.resource::<ActiveCombatEffects>().death_pops().len(),
                0
            );
            if target.lethal() {
                assert!(world.get_entity(evidence.target).is_err());
            } else {
                assert!(
                    world.get::<Health>(evidence.target).unwrap().current
                        < Health::default().current
                );
                assert_eq!(
                    world.get::<Transform>(evidence.target),
                    Some(&evidence.target_root)
                );
                assert_eq!(sprite_color(world, evidence.target), Color::WHITE);
            }
            world.resource_mut::<Time<Virtual>>().pause();
            world.resource_mut::<CombatEvidence>().phase = Phase::FlightCaptured;
            return capture(target, "flight");
        }
        Phase::FlightCaptured => {
            // Animate the resolved hit without advancing gameplay during evidence capture.
            let mut fixed = world.resource_mut::<Time<Fixed>>();
            fixed.discard_overstep(Duration::MAX);
            fixed.set_timestep(Duration::from_secs(60 * 60));
            world.resource_mut::<Time<Virtual>>().unpause();
            world.resource_mut::<CombatEvidence>().phase = Phase::Arrival;
        }
        Phase::Arrival => {
            if !world.resource::<ActiveCombatBolts>().is_empty() {
                assert!(ctx.frame < 20, "the bolt should arrive within 90 ms");
                return TestFlow::Continue;
            }
            let effects = world.resource::<ActiveCombatEffects>();
            if target.lethal() {
                let mut pops = effects.death_pops();
                let pop = pops.next().expect("death should start on arrival");
                assert_eq!(pop.victim.entity, evidence.target);
                assert!(
                    pop.victim
                        .position
                        .abs_diff_eq(evidence.target_root.translation.truncate(), 0.01)
                );
                assert_eq!(pops.len(), 0);
                assert_eq!(effects.decal_count(), 1);
            } else {
                assert_ne!(sprite_color(world, evidence.target), Color::WHITE);
                assert_eq!(
                    world.get::<Transform>(evidence.target),
                    Some(&evidence.target_root)
                );
                assert_eq!(effects.particles().len(), 6);
            }
            world.resource_mut::<CombatEvidence>().phase = Phase::Aftermath;
        }
        Phase::Aftermath => {
            if target.lethal() {
                let effects = world.resource::<ActiveCombatEffects>();
                let pop = effects
                    .death_pops()
                    .next()
                    .expect("the death pop should still be visible");
                if pop.elapsed_secs() < 0.045 {
                    return TestFlow::Continue;
                }
            }
            world.resource_mut::<Time<Virtual>>().pause();
            world.resource_mut::<CombatEvidence>().phase = Phase::Exit;
            return capture(
                target,
                if target.lethal() {
                    "destroyed"
                } else {
                    "impact"
                },
            );
        }
        Phase::Exit => return TestFlow::Exit,
    }
    TestFlow::Continue
}

fn capture(target: Target, stage: &str) -> TestFlow {
    TestFlow::Screenshot(format!("{}_{stage}", target.capture_prefix()))
}

fn sprite_color(world: &World, root: Entity) -> Color {
    let visual = world
        .get::<Children>(root)
        .unwrap()
        .iter()
        .find(|&child| world.get::<NanobotVisual>(child).is_some())
        .unwrap();
    world.get::<Sprite>(visual).unwrap().color
}

fn setup_scene(world: &mut World, target: Target) {
    world.insert_resource(fixed_simulation_time());
    world.resource_mut::<Time<Virtual>>().pause();
    clear_nanobots_and_sprite_entities(world);

    let center = Vec2::new(0.5, 5.5) * ZONE_BLOCK_SIZE;
    let (mut transform, mut projection, mut zoom) = world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .single_mut(world)
        .unwrap();
    transform.translation = center.extend(transform.translation.z);
    zoom.zoom = 1.0;
    let Projection::Orthographic(orthographic) = &mut *projection else {
        panic!("combat evidence expects an orthographic camera");
    };
    orthographic.scale = zoom.zoom;
    for entity in world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }

    let (opponent, &opponent_id) = world
        .query_filtered::<(Entity, &SwarmId), With<OpponentSwarm>>()
        .single(world)
        .unwrap();
    let attacker_root =
        Transform::from_translation((center + Vec2::new(-55.0, 0.0)).extend(GAMEPLAY_SPRITE_Z));
    let attacker = world
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            Health::default(),
            Charge::default(),
            SwarmMember::new(SwarmId::PLAYER),
            DefenderAttackCooldown::new(u32::MAX),
            attacker_root,
        ))
        .id();
    let target_root =
        Transform::from_translation((center + Vec2::new(55.0, 0.0)).extend(GAMEPLAY_SPRITE_Z));
    let target = match target {
        Target::Defender | Target::Worker => {
            let kind = if target.lethal() {
                NanobotType::Worker
            } else {
                NanobotType::Defender
            };
            let mut health = Health::default();
            if target.lethal() {
                health.current = 1;
            }
            world
                .spawn((
                    Nanobot {},
                    kind,
                    Commitment::Idle,
                    health,
                    Charge::default(),
                    SwarmMember::new(opponent_id),
                    DefenderAttackCooldown::new(u32::MAX),
                    target_root,
                ))
                .id()
        }
        Target::Facility => {
            let kind = PlannedKind::ProductionFacility;
            let mut sprite = world
                .resource::<StructureSprites>()
                .sprite(kind, StructureVisualState::Completed);
            sprite.color = completed_visual_color();
            sprite.custom_size = Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT));
            let mut structure = Structure::new(StructureKind::Basic);
            structure.health = 1;
            world
                .spawn((
                    structure,
                    OwnerSwarm(opponent),
                    StructureVisual::completed(kind),
                    sprite,
                    target_root,
                ))
                .id()
        }
    };
    world.insert_resource(CombatEvidence {
        attacker,
        attacker_root,
        target,
        target_root,
        phase: Phase::Ready,
    });
}
