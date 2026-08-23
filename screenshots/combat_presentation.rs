//! Offscreen evidence for resolved Defender hits, recovery, and nanobot destruction.

use std::time::Duration;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActiveCombatPulses, ActiveNanobotDeathGhosts, Charge, CombatPresentationSettings,
        Commitment, DefendHold, Health, Nanobot, NanobotDeathGhost, NanobotType, NanobotVisual,
        OpponentSwarm, Swarm, SwarmId, SwarmMember,
    },
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
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = position.x;
        transform.translation.y = position.y;
        zoom.zoom = 0.5;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = 0.5;
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
