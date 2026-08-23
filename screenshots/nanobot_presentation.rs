//! Offscreen evidence for the generic nanobot presentation-child cutover.

use std::collections::HashMap;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        Nanobot, NanobotSprites, NanobotType, NanobotVisual, OpponentSwarm, Swarm, SwarmId,
        SwarmMember, VelocityComponent,
    },
};

use crate::harness::{TestContext, TestFlow};

const CENTER: Vec2 = Vec2::new(1024.0, 3000.0);
const COLUMN_X: [f32; 3] = [-180.0, 0.0, 180.0];
const ROW_Y: [f32; 2] = [100.0, -100.0];
const ROTATIONS: [f32; 3] = [-0.65, 0.0, 0.65];

#[derive(Debug, Clone, Copy)]
struct ExpectedVisual {
    root: Entity,
    kind: NanobotType,
    swarm: SwarmId,
    position: Vec3,
    rotation: Quat,
}

#[derive(Resource)]
struct EvidenceLayout {
    opponent: SwarmId,
    visuals: Vec<ExpectedVisual>,
}

fn prepare_evidence(world: &mut World) {
    world.resource_mut::<Time<Virtual>>().pause();

    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = CENTER.x;
        transform.translation.y = CENTER.y;
        zoom.zoom = 0.55;
        if let Projection::Orthographic(orthographic) = &mut *projection {
            orthographic.scale = 0.55;
        }
    }
    for entity in world
        .query_filtered::<Entity, With<Node>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }

    let opponent = world
        .query_filtered::<&SwarmId, (With<Swarm>, With<OpponentSwarm>)>()
        .single(world)
        .expect("authored opponent swarm must exist")
        .to_owned();
    let roots = world
        .query_filtered::<(Entity, &NanobotType, &SwarmMember), With<Nanobot>>()
        .iter(world)
        .map(|(entity, kind, member)| (entity, *kind, member.0))
        .collect::<Vec<_>>();
    let mut representatives = HashMap::<(SwarmId, NanobotType), Entity>::new();
    for (entity, kind, swarm) in &roots {
        representatives.entry((*swarm, *kind)).or_insert(*entity);
    }

    let mut visuals = Vec::new();
    for (row, swarm) in [SwarmId::PLAYER, opponent].into_iter().enumerate() {
        for (column, kind) in NanobotType::ALL.into_iter().enumerate() {
            let root = *representatives
                .get(&(swarm, kind))
                .expect("authored scenario must contain every type for both factions");
            let position =
                (CENTER + Vec2::new(COLUMN_X[column], ROW_Y[row])).extend(GAMEPLAY_SPRITE_Z);
            let rotation = Quat::from_rotation_z(ROTATIONS[column]);
            world.entity_mut(root).insert((
                Transform::from_translation(position).with_rotation(rotation),
                Visibility::Visible,
            ));
            world.entity_mut(root).remove::<VelocityComponent>();
            visuals.push(ExpectedVisual {
                root,
                kind,
                swarm,
                position,
                rotation,
            });
        }
    }
    for (root, _, _) in roots {
        if !visuals.iter().any(|visual| visual.root == root) {
            world.entity_mut(root).insert(Visibility::Hidden);
        }
    }

    world.insert_resource(EvidenceLayout { opponent, visuals });
}

fn assert_evidence(world: &mut World) {
    let sprites = world.resource::<NanobotSprites>().clone();
    let layout = world.resource::<EvidenceLayout>();
    for expected in &layout.visuals {
        let root = world.entity(expected.root);
        assert!(root.get::<ChildOf>().is_none());
        assert!(root.get::<Sprite>().is_none());
        assert_eq!(root.get::<Visibility>(), Some(&Visibility::Visible));
        let root_transform = root
            .get::<Transform>()
            .expect("evidence root needs its gameplay Transform");
        assert!(
            root_transform
                .translation
                .abs_diff_eq(expected.position, 0.001),
            "frozen gameplay root moved away from the evidence grid: actual={:?}, expected={:?}",
            root_transform.translation,
            expected.position
        );
        assert!(
            root_transform.rotation.angle_between(expected.rotation) < 0.001,
            "frozen gameplay root changed its evidence orientation"
        );

        let visual = root
            .get::<Children>()
            .expect("evidence root must own a child")
            .iter()
            .find(|child| world.get::<NanobotVisual>(*child).is_some())
            .expect("evidence root must own its marked presentation child");
        let visual_entity = world.entity(visual);
        assert_eq!(
            visual_entity
                .get::<Sprite>()
                .expect("presentation child needs a Sprite")
                .image,
            sprites.handle(expected.kind, expected.swarm == layout.opponent)
        );
        let local = visual_entity
            .get::<Transform>()
            .expect("presentation child needs a local Transform");
        assert_eq!(local.translation, Vec3::ZERO);
        assert_eq!(local.rotation, Quat::IDENTITY);
        assert_eq!(local.scale, Vec3::ONE);

        let propagated = visual_entity
            .get::<GlobalTransform>()
            .expect("presentation child pose must propagate")
            .compute_transform();
        assert!(
            propagated.translation.abs_diff_eq(expected.position, 0.001),
            "visual child must remain centered on its gameplay root: actual={:?}, expected={:?}",
            propagated.translation,
            expected.position
        );
        assert!(
            propagated.rotation.angle_between(expected.rotation) < 0.001,
            "visual child must inherit gameplay-root orientation"
        );
    }
}

pub fn nanobot_presentation(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        prepare_evidence(ctx.world);
        return TestFlow::Continue;
    }
    if ctx.frame == 1 {
        assert_evidence(ctx.world);
        return TestFlow::Screenshot("nanobot_presentation".to_string());
    }
    TestFlow::Exit
}
