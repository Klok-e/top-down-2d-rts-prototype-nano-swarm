//! Offscreen evidence of textured structure edges and body clearance on the fine grid.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{Nanobot, NanobotType, PlannedKind, SwarmId, SwarmMember},
    navigation::{Obstacle, align_structure},
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

#[derive(Resource)]
struct Evidence {
    structures: Vec<(Entity, Vec2, Vec2)>,
    bots: Vec<(Entity, Vec2)>,
}

pub fn navigation_geometry(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        prepare(ctx.world);
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        let evidence = ctx.world.resource::<Evidence>();
        for (entity, expected_min, expected_size) in &evidence.structures {
            let transform = ctx.world.get::<Transform>(*entity).unwrap();
            let sprite = ctx.world.get::<Sprite>(*entity).unwrap();
            let size = sprite.custom_size.unwrap() * transform.scale.truncate();
            assert!(size.abs_diff_eq(*expected_size, 0.001));
            assert!(
                (transform.translation.truncate() - size / 2.0).abs_diff_eq(*expected_min, 0.001),
                "textured structure edges must coincide with the 72-unit grid"
            );
            for (bot, position) in &evidence.bots {
                assert!(Obstacle::structure(transform).admits_body(*position));
                assert!(
                    ctx.world
                        .get::<Transform>(*bot)
                        .unwrap()
                        .translation
                        .truncate()
                        .abs_diff_eq(*position, 0.001),
                    "stationary body must retain its place in the one-cell passage"
                );
            }
        }
        return TestFlow::Screenshot("navigation_geometry".to_owned());
    }
    if ctx.frame < 2 {
        TestFlow::Continue
    } else {
        TestFlow::Exit
    }
}

fn prepare(world: &mut World) {
    world.resource_mut::<Time<Virtual>>().pause();
    clear_nanobots_and_sprite_entities(world);
    for entity in world
        .query_filtered::<Entity, Or<(With<Mesh2d>, With<Node>)>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        let _ = world.despawn(entity);
    }
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 36.0;
        transform.translation.y = 288.0;
        zoom.zoom = 1.0;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 1.0;
        }
    }
    rect(
        world,
        Vec2::new(36.0, 288.0),
        Vec2::new(1200.0, 700.0),
        -1.0,
        Color::srgb(0.035, 0.045, 0.065),
    );
    for x in -4..=5 {
        rect(
            world,
            Vec2::new(x as f32 * 72.0, 288.0),
            Vec2::new(1.0, 576.0),
            0.0,
            Color::srgb(0.25, 0.3, 0.36),
        );
    }
    for y in 0..=8 {
        rect(
            world,
            Vec2::new(36.0, y as f32 * 72.0),
            Vec2::new(648.0, 1.0),
            0.0,
            Color::srgb(0.25, 0.3, 0.36),
        );
    }
    label(world, "COMPLETED", Vec2::new(-180.0, 615.0), 22.0);
    label(world, "PLANNED", Vec2::new(230.0, 615.0), 22.0);
    label(
        world,
        "72-unit cells | 68-unit bodies",
        Vec2::new(36.0, -35.0),
        20.0,
    );

    let sprites = world.resource::<StructureSprites>().clone();
    let mut structures = Vec::new();
    for (row, (kind, name)) in [
        (PlannedKind::SourceStockpile, "Source"),
        (PlannedKind::SinkStockpile, "Sink"),
        (PlannedKind::ProductionFacility, "Production"),
        (PlannedKind::Charger, "Charger"),
    ]
    .into_iter()
    .enumerate()
    {
        let y = row as f32 * 144.0;
        label(world, name, Vec2::new(-260.0, y + 72.0), 18.0);
        for (state, authored_center, authored_scale, expected_min, expected_size) in [
            (
                StructureVisualState::Completed,
                Vec2::new(-70.0, y + 70.0),
                Vec2::new(2.2, 2.3),
                Vec2::new(-144.0, y),
                Vec2::splat(144.0),
            ),
            (
                StructureVisualState::Planned,
                Vec2::new(110.0, y + 70.0),
                Vec2::new(1.2, 2.3),
                Vec2::new(72.0, y),
                Vec2::new(72.0, 144.0),
            ),
        ] {
            let transform = align_structure(
                Transform::from_translation(authored_center.extend(GAMEPLAY_SPRITE_Z))
                    .with_scale(authored_scale.extend(1.0)),
            );
            let mut sprite = sprites.sprite(kind, state);
            sprite.custom_size = Some(Vec2::splat(64.0));
            let entity = world
                .spawn((sprite, transform, StructureVisual { kind, state }))
                .id();
            structures.push((entity, expected_min, expected_size));
        }
    }
    // The passage spans x=0..72; all three rendered bodies are centered at x=36.
    let mut bots = Vec::new();
    for (kind, y) in [
        (NanobotType::Worker, 108.0),
        (NanobotType::Hauler, 252.0),
        (NanobotType::Defender, 396.0),
    ] {
        let position = Vec2::new(36.0, y);
        let entity = world
            .spawn((
                kind,
                SwarmMember(SwarmId::PLAYER),
                Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z + 1.0)),
            ))
            .id();
        world.entity_mut(entity).insert(Nanobot {});
        bots.push((entity, position));
    }
    world.insert_resource(Evidence { structures, bots });
}

fn rect(world: &mut World, center: Vec2, size: Vec2, z: f32, color: Color) {
    world.spawn((
        Sprite::from_color(color, size),
        Transform::from_translation(center.extend(z)),
    ));
}

fn label(world: &mut World, text: &str, position: Vec2, size: f32) {
    world.spawn((
        Text2d::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb(0.8, 0.85, 0.9)),
        Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z + 2.0)),
    ));
}
