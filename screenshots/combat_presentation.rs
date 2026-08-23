//! Offscreen evidence for resolved Defender hits, recovery, and nanobot destruction.

use std::time::Duration;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActiveCombatDecorations, ActiveCombatPulses, ActiveNanobotDeathGhosts,
        ActiveStructureDeathGhosts, Charge, CombatPresentationSettings, Commitment, DefendHold,
        DefenderAttackCooldown, Health, Nanobot, NanobotDeathGhost, NanobotType, NanobotVisual,
        OpponentSwarm, OwnerSwarm, PLANNED_STRUCTURE_FOOTPRINT, PlannedKind, Structure,
        StructureKind, Swarm, SwarmId, SwarmMember, completed_visual_color,
    },
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

const SCENE_CELL: IVec2 = IVec2::new(0, 5);
const HOLD_CELL: IVec2 = IVec2::ZERO;
const ZONE_BLOCK_SIZE: f32 = top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE;

#[derive(Clone, Copy, Resource)]
struct CombatEvidence {
    attacker: Entity,
    target: Entity,
    attacker_visual: Entity,
    target_visual: Entity,
    attacker_root: Transform,
    target_root: Transform,
    target_start_health: u32,
    recovered_capture_requested: bool,
}

fn cell_center(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

fn visual_child(world: &World, root: Entity) -> Entity {
    world
        .get::<Children>(root)
        .expect("combat evidence nanobot needs children")
        .iter()
        .find(|child| world.get::<NanobotVisual>(*child).is_some())
        .expect("combat evidence nanobot needs a presentation child")
}

fn focus_camera(world: &mut World, position: Vec2) {
    focus_camera_at_zoom(world, position, 0.5);
}

fn focus_camera_at_zoom(world: &mut World, position: Vec2, value: f32) {
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = position.x;
        transform.translation.y = position.y;
        zoom.zoom = value;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = value;
        }
    }
}

fn prepare_combat_scene(world: &mut World) -> Vec2 {
    world.resource_mut::<Time<Virtual>>().pause();
    clear_nanobots_and_sprite_entities(world);
    for entity in world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }

    let center = cell_center(SCENE_CELL);
    focus_camera(world, center);
    center
}

fn setup_scene(world: &mut World) {
    let center = prepare_combat_scene(world);
    world.resource_mut::<IntentGrid>().paint_owned(
        HOLD_CELL,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let opponent = world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(world)
        .expect("authored scene needs an Opponent Swarm")
        .to_owned();
    let attacker_position = center + Vec2::new(-44.0, 0.0);
    let target_position = center + Vec2::new(44.0, 0.0);
    let attacker_root = Transform::from_translation(attacker_position.extend(GAMEPLAY_SPRITE_Z));
    let target_root = Transform::from_translation(target_position.extend(GAMEPLAY_SPRITE_Z));
    let attacker = world
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            Health::default(),
            Charge::default(),
            SwarmMember::new(SwarmId::PLAYER),
            attacker_root,
        ))
        .id();
    let target = world
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            Health::default(),
            Charge::default(),
            SwarmMember::new(opponent),
            target_root,
        ))
        .id();
    world.insert_resource(CombatEvidence {
        attacker,
        target,
        attacker_visual: Entity::PLACEHOLDER,
        target_visual: Entity::PLACEHOLDER,
        attacker_root,
        target_root,
        target_start_health: Health::default().current,
        recovered_capture_requested: false,
    });
}

fn assert_roots_unchanged(world: &World, evidence: &CombatEvidence) {
    assert_eq!(
        world.get::<Transform>(evidence.attacker),
        Some(&evidence.attacker_root),
    );
    assert_eq!(
        world.get::<Transform>(evidence.target),
        Some(&evidence.target_root),
    );
}

fn is_neutral(world: &World, evidence: &CombatEvidence) -> bool {
    world.get::<Transform>(evidence.attacker_visual) == Some(&Transform::IDENTITY)
        && world.get::<Transform>(evidence.target_visual) == Some(&Transform::IDENTITY)
        && world
            .get::<Sprite>(evidence.target_visual)
            .is_some_and(|sprite| sprite.color == Color::WHITE)
}

pub fn combat_presentation(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        setup_scene(ctx.world);
        return TestFlow::Continue;
    }

    if ctx.frame == 1 {
        let attacker = ctx.world.resource::<CombatEvidence>().attacker;
        let target = ctx.world.resource::<CombatEvidence>().target;
        let attacker_visual = visual_child(ctx.world, attacker);
        let target_visual = visual_child(ctx.world, target);
        {
            let mut evidence = ctx.world.resource_mut::<CombatEvidence>();
            evidence.attacker_visual = attacker_visual;
            evidence.target_visual = target_visual;
        }
        let evidence = *ctx.world.resource::<CombatEvidence>();
        assert_roots_unchanged(ctx.world, &evidence);
        assert!(is_neutral(ctx.world, &evidence));
        assert!(ctx.world.resource::<ActiveCombatPulses>().is_empty());
        return TestFlow::Screenshot("combat_presentation_neutral".to_string());
    }

    if ctx.frame == 2 {
        let attacker = ctx.world.resource::<CombatEvidence>().attacker;
        ctx.world
            .entity_mut(attacker)
            .insert(DefendHold { cell: HOLD_CELL });
        ctx.world.resource_mut::<Time<Virtual>>().unpause();
        return TestFlow::Continue;
    }

    if ctx.frame == 3 {
        return TestFlow::Continue;
    }

    if ctx.frame == 4 {
        ctx.world.resource_mut::<Time<Virtual>>().pause();
        let evidence = *ctx.world.resource::<CombatEvidence>();
        assert!(
            ctx.world
                .get::<Health>(evidence.target)
                .expect("non-lethal target must remain")
                .current
                < evidence.target_start_health,
            "the impact artifact must follow real fixed-step damage",
        );
        assert_roots_unchanged(ctx.world, &evidence);
        assert!(
            ctx.world
                .get::<Transform>(evidence.attacker_visual)
                .expect("attacker visual must remain")
                .translation
                .length()
                > 0.0,
        );
        assert!(
            ctx.world
                .get::<Transform>(evidence.target_visual)
                .expect("target visual must remain")
                .translation
                .length()
                > 0.0,
        );
        assert_ne!(
            ctx.world
                .get::<Sprite>(evidence.target_visual)
                .expect("target visual needs a Sprite")
                .color,
            Color::WHITE,
        );
        let pulse = ctx
            .world
            .resource::<ActiveCombatPulses>()
            .iter()
            .copied()
            .next()
            .expect("one real resolved hit must render one pulse");
        assert_eq!(ctx.world.resource::<ActiveCombatPulses>().len(), 1);
        let pulse_color = pulse.color.to_srgba();
        assert!(pulse_color.blue > pulse_color.red);
        return TestFlow::Screenshot("combat_presentation_impact".to_string());
    }

    if ctx.frame == 5 {
        let mut fixed = ctx.world.resource_mut::<Time<Fixed>>();
        fixed.discard_overstep(Duration::MAX);
        fixed.set_timestep(Duration::from_secs(60 * 60));
        ctx.world.resource_mut::<Time<Virtual>>().unpause();
        return TestFlow::Continue;
    }

    if ctx
        .world
        .resource::<CombatEvidence>()
        .recovered_capture_requested
    {
        return TestFlow::Exit;
    }

    let evidence = *ctx.world.resource::<CombatEvidence>();
    assert_roots_unchanged(ctx.world, &evidence);
    if ctx.world.resource::<ActiveCombatPulses>().is_empty() && is_neutral(ctx.world, &evidence) {
        ctx.world.resource_mut::<Time<Virtual>>().pause();
        ctx.world
            .resource_mut::<CombatEvidence>()
            .recovered_capture_requested = true;
        return TestFlow::Screenshot("combat_presentation_recovered".to_string());
    }
    assert!(ctx.frame < 30, "combat visuals did not recover on schedule");
    TestFlow::Continue
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LethalEvidencePhase {
    AwaitVisuals,
    AwaitImpact,
    ImpactCaptured,
    AwaitDissolve,
    DissolveCaptured,
    AwaitExpiry,
    ExpiryCaptured,
}

#[derive(Clone, Copy, Resource)]
struct LethalCombatEvidence {
    attacker: Entity,
    victim: Entity,
    attacker_visual: Entity,
    attacker_root: Transform,
    victim_position: Vec2,
    phase: LethalEvidencePhase,
}

fn setup_lethal_scene(world: &mut World) {
    let center = prepare_combat_scene(world);
    world.resource_mut::<IntentGrid>().paint_owned(
        HOLD_CELL,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let opponent = *world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(world)
        .expect("authored scene needs an Opponent Swarm");
    let attacker_position = center + Vec2::new(-44.0, 0.0);
    let victim_position = center + Vec2::new(44.0, 0.0);
    let attacker_root = Transform::from_translation(attacker_position.extend(GAMEPLAY_SPRITE_Z));
    let attacker = world
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            Health::default(),
            Charge::default(),
            SwarmMember::new(SwarmId::PLAYER),
            attacker_root,
        ))
        .id();
    let victim = world
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            Commitment::Idle,
            Health {
                current: 10,
                max: Health::default().max,
            },
            SwarmMember::new(opponent),
            Transform::from_translation(victim_position.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id();
    world.insert_resource(LethalCombatEvidence {
        attacker,
        victim,
        attacker_visual: Entity::PLACEHOLDER,
        attacker_root,
        victim_position,
        phase: LethalEvidencePhase::AwaitVisuals,
    });
}

fn death_ghosts(world: &World) -> Vec<NanobotDeathGhost> {
    world
        .resource::<ActiveNanobotDeathGhosts>()
        .iter()
        .cloned()
        .collect()
}

pub fn nanobot_combat_death(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        setup_lethal_scene(ctx.world);
        return TestFlow::Continue;
    }

    let evidence = *ctx.world.resource::<LethalCombatEvidence>();
    match evidence.phase {
        LethalEvidencePhase::AwaitVisuals => {
            let attacker_visual = visual_child(ctx.world, evidence.attacker);
            ctx.world
                .entity_mut(evidence.attacker)
                .insert(DefendHold { cell: HOLD_CELL });
            let mut next = ctx.world.resource_mut::<LethalCombatEvidence>();
            next.attacker_visual = attacker_visual;
            next.phase = LethalEvidencePhase::AwaitImpact;
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            TestFlow::Continue
        }
        LethalEvidencePhase::AwaitImpact => {
            if ctx.world.entities().contains(evidence.victim) {
                assert!(ctx.frame < 12, "lethal combat did not resolve on schedule");
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            assert_eq!(
                ctx.world.get::<Transform>(evidence.attacker),
                Some(&evidence.attacker_root),
            );
            assert!(
                ctx.world
                    .get::<Transform>(evidence.attacker_visual)
                    .expect("lethal attacker keeps its visual")
                    .translation
                    .length()
                    > 0.0,
            );
            assert_eq!(ctx.world.resource::<ActiveCombatPulses>().len(), 1);
            let ghosts = death_ghosts(ctx.world);
            let [ghost] = ghosts.as_slice() else {
                panic!("lethal impact needs one nanobot death ghost: {ghosts:?}");
            };
            assert_eq!(ghost.victim.entity, evidence.victim);
            assert_eq!(ghost.victim.position, evidence.victim_position);
            assert_eq!(
                ghost.transform.translation.truncate(),
                evidence.victim_position
            );
            assert_ne!(ghost.color, Color::WHITE);
            ctx.world.resource_mut::<LethalCombatEvidence>().phase =
                LethalEvidencePhase::ImpactCaptured;
            TestFlow::Screenshot("nanobot_combat_death_impact".to_string())
        }
        LethalEvidencePhase::ImpactCaptured => {
            let mut fixed = ctx.world.resource_mut::<Time<Fixed>>();
            fixed.discard_overstep(Duration::MAX);
            fixed.set_timestep(Duration::from_secs(60 * 60));
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            ctx.world.resource_mut::<LethalCombatEvidence>().phase =
                LethalEvidencePhase::AwaitDissolve;
            TestFlow::Continue
        }
        LethalEvidencePhase::AwaitDissolve => {
            let ghosts = death_ghosts(ctx.world);
            let Some(ghost) = ghosts.first() else {
                panic!("nanobot death ghost expired before its dissolve evidence");
            };
            let alpha = ghost.color.to_srgba().alpha;
            if ghost.transform.scale.y < 0.7 && alpha < 0.9 {
                ctx.world.resource_mut::<Time<Virtual>>().pause();
                assert!(ctx.world.resource::<ActiveCombatPulses>().is_empty());
                ctx.world.resource_mut::<LethalCombatEvidence>().phase =
                    LethalEvidencePhase::DissolveCaptured;
                return TestFlow::Screenshot("nanobot_combat_death_dissolve".to_string());
            }
            let duration = ctx
                .world
                .resource::<CombatPresentationSettings>()
                .death_duration;
            assert!(
                ctx.frame < 12 + duration.as_millis() as u32 / 10,
                "nanobot death ghost did not enter its dissolve phase",
            );
            TestFlow::Continue
        }
        LethalEvidencePhase::DissolveCaptured => {
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            ctx.world.resource_mut::<LethalCombatEvidence>().phase =
                LethalEvidencePhase::AwaitExpiry;
            TestFlow::Continue
        }
        LethalEvidencePhase::AwaitExpiry => {
            if !death_ghosts(ctx.world).is_empty() {
                assert!(ctx.frame < 60, "nanobot death ghost did not expire");
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            assert!(!ctx.world.entities().contains(evidence.victim));
            assert!(ctx.world.resource::<ActiveCombatPulses>().is_empty());
            assert_eq!(
                ctx.world.get::<Transform>(evidence.attacker),
                Some(&evidence.attacker_root),
            );
            assert_eq!(
                ctx.world.get::<Transform>(evidence.attacker_visual),
                Some(&Transform::IDENTITY),
            );
            ctx.world.resource_mut::<LethalCombatEvidence>().phase =
                LethalEvidencePhase::ExpiryCaptured;
            TestFlow::Screenshot("nanobot_combat_death_expired".to_string())
        }
        LethalEvidencePhase::ExpiryCaptured => TestFlow::Exit,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DenseCombatPhase {
    AwaitVisuals,
    AwaitImpact,
    ImpactCaptured,
    AwaitNearBoundaryRecovery,
    AwaitNearBoundary,
    NearBoundaryCaptured,
    AwaitTacticalBoundary,
    TacticalBoundaryCaptured,
}

#[derive(Resource)]
struct DenseCombatEvidence {
    attackers: Vec<(Entity, Transform)>,
    targets: Vec<(Entity, Transform)>,
    phase: DenseCombatPhase,
}

fn spawn_dense_nanobot(
    world: &mut World,
    position: Vec2,
    swarm: SwarmId,
    kind: NanobotType,
) -> (Entity, Transform) {
    let root = Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z));
    let entity = world
        .spawn((
            Nanobot {},
            kind,
            Commitment::Idle,
            Health::default(),
            SwarmMember::new(swarm),
            root,
        ))
        .id();
    if kind == NanobotType::Defender {
        world.entity_mut(entity).insert(Charge::default());
    }
    (entity, root)
}

fn setup_dense_scene(world: &mut World) {
    let center = prepare_combat_scene(world);
    focus_camera_at_zoom(world, center, 1.0);
    let opponent = *world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(world)
        .expect("authored scene needs an Opponent Swarm");
    world.resource_mut::<IntentGrid>().paint_owned(
        HOLD_CELL,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    world
        .resource_mut::<IntentGrid>()
        .contest_defend(HOLD_CELL, opponent);
    let left_target_position = center + Vec2::new(-180.0, 0.0);
    let right_target_position = center + Vec2::new(180.0, 0.0);
    let targets = vec![
        spawn_dense_nanobot(world, left_target_position, opponent, NanobotType::Worker),
        spawn_dense_nanobot(
            world,
            right_target_position,
            SwarmId::PLAYER,
            NanobotType::Worker,
        ),
    ];
    let mut attackers = Vec::new();
    for offset in [
        Vec2::new(-64.0, -32.0),
        Vec2::new(-64.0, 32.0),
        Vec2::new(0.0, -64.0),
    ] {
        attackers.push(spawn_dense_nanobot(
            world,
            left_target_position + offset,
            SwarmId::PLAYER,
            NanobotType::Defender,
        ));
    }
    for offset in [
        Vec2::new(64.0, -32.0),
        Vec2::new(64.0, 32.0),
        Vec2::new(0.0, -64.0),
    ] {
        attackers.push(spawn_dense_nanobot(
            world,
            right_target_position + offset,
            opponent,
            NanobotType::Defender,
        ));
    }
    world.insert_resource(DenseCombatEvidence {
        attackers,
        targets,
        phase: DenseCombatPhase::AwaitVisuals,
    });
}

fn assert_dense_roots_unchanged(world: &World, evidence: &DenseCombatEvidence) {
    for (entity, root) in evidence.attackers.iter().chain(&evidence.targets) {
        assert_eq!(world.get::<Transform>(*entity), Some(root));
    }
}

pub fn combat_presentation_density_and_zoom(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        setup_dense_scene(ctx.world);
        return TestFlow::Continue;
    }

    let phase = ctx.world.resource::<DenseCombatEvidence>().phase;
    match phase {
        DenseCombatPhase::AwaitVisuals => {
            let attackers = ctx
                .world
                .resource::<DenseCombatEvidence>()
                .attackers
                .iter()
                .map(|(entity, _)| *entity)
                .collect::<Vec<_>>();
            for attacker in attackers {
                let _ = visual_child(ctx.world, attacker);
                ctx.world
                    .entity_mut(attacker)
                    .insert(DefendHold { cell: HOLD_CELL });
            }
            for (target, _) in &ctx.world.resource::<DenseCombatEvidence>().targets {
                let _ = visual_child(ctx.world, *target);
            }
            ctx.world.resource_mut::<DenseCombatEvidence>().phase = DenseCombatPhase::AwaitImpact;
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            TestFlow::Continue
        }
        DenseCombatPhase::AwaitImpact => {
            let pulse_count = ctx.world.resource::<ActiveCombatPulses>().len();
            if pulse_count != 6 {
                let target_health = ctx
                    .world
                    .resource::<DenseCombatEvidence>()
                    .targets
                    .iter()
                    .map(|(target, _)| {
                        ctx.world
                            .get::<Health>(*target)
                            .map(|health| health.current)
                    })
                    .collect::<Vec<_>>();
                assert!(
                    ctx.frame < 12,
                    "dense combat did not resolve on schedule: {pulse_count} pulses, target health {target_health:?}",
                );
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            let evidence = ctx.world.resource::<DenseCombatEvidence>();
            assert_dense_roots_unchanged(ctx.world, evidence);
            let settings = *ctx.world.resource::<CombatPresentationSettings>();
            for (target, _) in &evidence.targets {
                assert_eq!(
                    ctx.world.get::<Health>(*target).unwrap().current,
                    70,
                    "three real attacks must retain their existing aggregate damage",
                );
                let visual = visual_child(ctx.world, *target);
                let reaction = ctx.world.get::<Transform>(visual).unwrap().translation;
                assert!(reaction.length() > 0.0);
                assert!(reaction.length() <= settings.recoil_distance + 0.001);
                assert_ne!(ctx.world.get::<Sprite>(visual).unwrap().color, Color::WHITE);
            }
            for (attacker, _) in &evidence.attackers {
                let visual = visual_child(ctx.world, *attacker);
                assert!(
                    ctx.world
                        .get::<Transform>(visual)
                        .unwrap()
                        .translation
                        .length()
                        > 0.0
                );
            }
            let mut player_pulses = 0;
            let mut opponent_pulses = 0;
            for pulse in ctx.world.resource::<ActiveCombatPulses>().iter() {
                let color = pulse.color.to_srgba();
                if color.blue > color.red {
                    player_pulses += 1;
                } else {
                    opponent_pulses += 1;
                }
            }
            assert_eq!((player_pulses, opponent_pulses), (3, 3));
            assert!(settings.pulse_thickness >= 1.0);
            assert!(
                ctx.world.resource::<ActiveCombatDecorations>().len()
                    <= settings.max_decorative_effects,
            );
            ctx.world.resource_mut::<DenseCombatEvidence>().phase =
                DenseCombatPhase::ImpactCaptured;
            TestFlow::Screenshot("combat_presentation_dense_volley".to_string())
        }
        DenseCombatPhase::ImpactCaptured => {
            let center = cell_center(SCENE_CELL);
            focus_camera_at_zoom(ctx.world, center, 7.99);
            ctx.world.resource_mut::<DenseCombatEvidence>().phase =
                DenseCombatPhase::AwaitNearBoundaryRecovery;
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            TestFlow::Continue
        }
        DenseCombatPhase::AwaitNearBoundaryRecovery => {
            if !ctx.world.resource::<ActiveCombatPulses>().is_empty() {
                assert!(
                    ctx.frame < 30,
                    "first dense volley did not expire on schedule"
                );
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<DenseCombatEvidence>().phase =
                DenseCombatPhase::AwaitNearBoundary;
            TestFlow::Continue
        }
        DenseCombatPhase::AwaitNearBoundary => {
            if ctx.world.resource::<ActiveCombatPulses>().len() != 6 {
                assert!(
                    ctx.frame < 40,
                    "near-boundary dense volley did not resolve on schedule",
                );
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            let evidence = ctx.world.resource::<DenseCombatEvidence>();
            assert_dense_roots_unchanged(ctx.world, evidence);
            let minimum = ctx
                .world
                .resource::<CombatPresentationSettings>()
                .minimum_impact_screen_distance;
            for (entity, _) in evidence.attackers.iter().chain(&evidence.targets) {
                let visual = visual_child(ctx.world, *entity);
                assert!(
                    ctx.world
                        .get::<Transform>(visual)
                        .unwrap()
                        .translation
                        .length()
                        / 7.99
                        >= minimum - 0.001,
                );
            }
            for (target, _) in &evidence.targets {
                assert_eq!(ctx.world.get::<Health>(*target).unwrap().current, 40);
            }
            ctx.world.resource_mut::<DenseCombatEvidence>().phase =
                DenseCombatPhase::NearBoundaryCaptured;
            TestFlow::Screenshot("combat_presentation_near_tactical_boundary".to_string())
        }
        DenseCombatPhase::NearBoundaryCaptured => {
            let center = cell_center(SCENE_CELL);
            focus_camera_at_zoom(ctx.world, center, 8.0);
            ctx.world.resource_mut::<DenseCombatEvidence>().phase =
                DenseCombatPhase::AwaitTacticalBoundary;
            TestFlow::Continue
        }
        DenseCombatPhase::AwaitTacticalBoundary => {
            assert_eq!(ctx.world.resource::<ActiveCombatPulses>().len(), 6);
            assert!(ctx.world.resource::<ActiveNanobotDeathGhosts>().is_empty());
            assert!(!ctx.world.resource::<ActiveCombatDecorations>().is_empty());
            let evidence = ctx.world.resource::<DenseCombatEvidence>();
            assert_dense_roots_unchanged(ctx.world, evidence);
            for (entity, _) in evidence.attackers.iter().chain(&evidence.targets) {
                let visual = visual_child(ctx.world, *entity);
                assert_eq!(
                    ctx.world.get::<Transform>(visual),
                    Some(&Transform::IDENTITY),
                );
                assert_eq!(ctx.world.get::<Sprite>(visual).unwrap().color, Color::WHITE);
            }
            ctx.world.resource_mut::<DenseCombatEvidence>().phase =
                DenseCombatPhase::TacticalBoundaryCaptured;
            TestFlow::Screenshot("combat_presentation_tactical_boundary".to_string())
        }
        DenseCombatPhase::TacticalBoundaryCaptured => TestFlow::Exit,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StructureCombatPhase {
    AwaitVisuals,
    AwaitOrdinaryHit,
    OrdinaryHitCaptured,
    AwaitRecovery,
    RecoveryCaptured,
    AwaitDestruction,
    DestructionCaptured,
}

#[derive(Clone, Copy, Resource)]
struct StructureCombatEvidence {
    attacker: Entity,
    attacker_visual: Entity,
    structure: Entity,
    attacker_root: Transform,
    structure_root: Transform,
    structure_position: Vec2,
    start_health: u32,
    phase: StructureCombatPhase,
}

fn setup_structure_combat_scene(world: &mut World) {
    let center = prepare_combat_scene(world);
    world.resource_mut::<IntentGrid>().paint_owned(
        HOLD_CELL,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    let opponent_entity = world
        .query_filtered::<Entity, (With<Swarm>, With<OpponentSwarm>)>()
        .single(world)
        .expect("authored scene needs an Opponent Swarm");
    let attacker_position = center + Vec2::new(-44.0, 0.0);
    let structure_position = center + Vec2::new(44.0, 0.0);
    let attacker_root = Transform::from_translation(attacker_position.extend(GAMEPLAY_SPRITE_Z));
    let structure_root = Transform::from_translation(structure_position.extend(GAMEPLAY_SPRITE_Z));
    let attacker = world
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Idle,
            Health::default(),
            Charge::default(),
            SwarmMember::new(SwarmId::PLAYER),
            attacker_root,
        ))
        .id();
    let kind = PlannedKind::ProductionFacility;
    let mut sprite = world
        .resource::<StructureSprites>()
        .sprite(kind, StructureVisualState::Completed);
    sprite.color = completed_visual_color();
    sprite.custom_size = Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT));
    let structure = world
        .spawn((
            Structure::new(StructureKind::Basic),
            OwnerSwarm(opponent_entity),
            StructureVisual::completed(kind),
            sprite,
            structure_root,
        ))
        .id();
    let start_health = world.get::<Structure>(structure).unwrap().health;
    world.insert_resource(StructureCombatEvidence {
        attacker,
        attacker_visual: Entity::PLACEHOLDER,
        structure,
        attacker_root,
        structure_root,
        structure_position,
        start_health,
        phase: StructureCombatPhase::AwaitVisuals,
    });
}

pub fn support_structure_combat_presentation(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        setup_structure_combat_scene(ctx.world);
        return TestFlow::Continue;
    }

    let evidence = *ctx.world.resource::<StructureCombatEvidence>();
    match evidence.phase {
        StructureCombatPhase::AwaitVisuals => {
            let attacker_visual = visual_child(ctx.world, evidence.attacker);
            ctx.world
                .entity_mut(evidence.attacker)
                .insert(DefendHold { cell: HOLD_CELL });
            let mut next = ctx.world.resource_mut::<StructureCombatEvidence>();
            next.attacker_visual = attacker_visual;
            next.phase = StructureCombatPhase::AwaitOrdinaryHit;
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            TestFlow::Continue
        }
        StructureCombatPhase::AwaitOrdinaryHit => {
            let health = ctx
                .world
                .get::<Structure>(evidence.structure)
                .expect("ordinary structure hit must not destroy the target")
                .health;
            if health == evidence.start_health {
                assert!(ctx.frame < 12, "ordinary structure hit did not resolve");
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            assert_eq!(ctx.world.resource::<ActiveCombatPulses>().len(), 1);
            assert!(
                ctx.world
                    .get::<Transform>(evidence.attacker_visual)
                    .unwrap()
                    .translation
                    .length()
                    > 0.0,
            );
            assert_ne!(
                ctx.world.get::<Sprite>(evidence.structure).unwrap().color,
                completed_visual_color(),
            );
            assert_eq!(
                ctx.world.get::<Transform>(evidence.structure),
                Some(&evidence.structure_root),
            );
            assert!(
                ctx.world
                    .resource::<ActiveStructureDeathGhosts>()
                    .is_empty(),
            );
            ctx.world.resource_mut::<StructureCombatEvidence>().phase =
                StructureCombatPhase::OrdinaryHitCaptured;
            TestFlow::Screenshot("support_structure_combat_hit".to_string())
        }
        StructureCombatPhase::OrdinaryHitCaptured => {
            let mut fixed = ctx.world.resource_mut::<Time<Fixed>>();
            fixed.discard_overstep(Duration::MAX);
            fixed.set_timestep(Duration::from_secs(60 * 60));
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            ctx.world.resource_mut::<StructureCombatEvidence>().phase =
                StructureCombatPhase::AwaitRecovery;
            TestFlow::Continue
        }
        StructureCombatPhase::AwaitRecovery => {
            let recovered = ctx.world.resource::<ActiveCombatPulses>().is_empty()
                && ctx
                    .world
                    .get::<Transform>(evidence.attacker_visual)
                    .is_some_and(|transform| *transform == Transform::IDENTITY)
                && ctx
                    .world
                    .get::<Sprite>(evidence.structure)
                    .is_some_and(|sprite| sprite.color == completed_visual_color());
            if !recovered {
                assert!(
                    ctx.frame < 40,
                    "structure impact did not recover on schedule"
                );
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            assert_eq!(
                ctx.world.get::<Transform>(evidence.structure),
                Some(&evidence.structure_root),
            );
            ctx.world.resource_mut::<StructureCombatEvidence>().phase =
                StructureCombatPhase::RecoveryCaptured;
            TestFlow::Screenshot("support_structure_combat_recovered".to_string())
        }
        StructureCombatPhase::RecoveryCaptured => {
            ctx.world
                .get_mut::<Structure>(evidence.structure)
                .expect("recovered structure must remain")
                .health = 1;
            ctx.world
                .entity_mut(evidence.attacker)
                .insert(DefenderAttackCooldown { ticks_remaining: 0 });
            let mut fixed = ctx.world.resource_mut::<Time<Fixed>>();
            fixed.discard_overstep(Duration::MAX);
            fixed.set_timestep(Duration::from_millis(10));
            ctx.world.resource_mut::<Time<Virtual>>().unpause();
            ctx.world.resource_mut::<StructureCombatEvidence>().phase =
                StructureCombatPhase::AwaitDestruction;
            TestFlow::Continue
        }
        StructureCombatPhase::AwaitDestruction => {
            if ctx.world.entities().contains(evidence.structure) {
                assert!(ctx.frame < 55, "lethal structure hit did not resolve");
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Time<Virtual>>().pause();
            assert_eq!(ctx.world.resource::<ActiveCombatPulses>().len(), 1);
            assert_eq!(
                ctx.world.get::<Transform>(evidence.attacker),
                Some(&evidence.attacker_root),
            );
            let ghost = ctx
                .world
                .resource::<ActiveStructureDeathGhosts>()
                .iter()
                .next()
                .cloned()
                .expect("lethal structure hit needs a presentation ghost and ring");
            assert_eq!(ghost.victim.entity, evidence.structure);
            assert_eq!(ghost.victim.position, evidence.structure_position);
            assert_eq!(
                ghost.transform.translation.truncate(),
                evidence.structure_position,
            );
            assert!(ghost.ring_radius > 0.0);
            assert!(ghost.ring_color.to_srgba().alpha > 0.0);
            ctx.world.resource_mut::<StructureCombatEvidence>().phase =
                StructureCombatPhase::DestructionCaptured;
            TestFlow::Screenshot("support_structure_combat_destroyed".to_string())
        }
        StructureCombatPhase::DestructionCaptured => TestFlow::Exit,
    }
}
