//! Screenshot evidence for bar-style structure and hauler fill indicators.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        Charger, HaulerLoad, Nanobot, NanobotType, PlannedKind, PlannedStructure,
        ProductionFacility,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
    structure_overlay::{StructureOverlay, StructureOverlayKind},
};

use crate::harness::{TestContext, TestFlow};

#[derive(Resource)]
struct FillIndicatorTargets(Vec<Entity>);

pub fn fill_indicators(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        despawn_existing_sprites(ctx.world);
        let targets = spawn_focused_scene(ctx.world);
        ctx.world.insert_resource(FillIndicatorTargets(targets));
        return TestFlow::Continue;
    }

    if ctx.frame < 12 {
        return TestFlow::Continue;
    }

    if ctx.frame == 12 {
        assert_bar_overlays_exist(ctx.world);
        return TestFlow::Screenshot("fill_indicators".to_string());
    }

    TestFlow::Exit
}

fn focus_camera(world: &mut World) {
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        zoom.zoom = 0.8;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.8;
        }
    }
}

fn despawn_existing_sprites(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Sprite>>()
        .iter(world)
        .collect();
    for entity in entities {
        let _ = world.despawn(entity);
    }
}

fn spawn_focused_scene(world: &mut World) -> Vec<Entity> {
    vec![
        spawn_deposit(world, Vec2::new(-225.0, -35.0)),
        spawn_stockpile(world, Vec2::new(-135.0, -35.0)),
        spawn_facility(world, Vec2::new(-45.0, -35.0)),
        spawn_planned(world, Vec2::new(45.0, -35.0)),
        spawn_charger(world, Vec2::new(135.0, -35.0)),
        spawn_loaded_hauler(world, Vec2::new(225.0, -35.0)),
    ]
}

fn spawn_deposit(world: &mut World, pos: Vec2) -> Entity {
    world
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 750,
                capacity: 1000,
                radius: 32.0,
            },
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(64.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_stockpile(world: &mut World, pos: Vec2) -> Entity {
    world
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 500,
                capacity: 1000,
                radius: 32.0,
            },
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_facility(world: &mut World, pos: Vec2) -> Entity {
    let mut facility = ProductionFacility::new();
    facility.input_amount = 120;
    facility.input_capacity = 200;
    world
        .spawn((
            facility,
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_planned(world: &mut World, pos: Vec2) -> Entity {
    let mut planned = PlannedStructure::new(PlannedKind::SinkStockpile, IVec2::ZERO);
    planned.work_remaining = 2;
    world
        .spawn((
            planned,
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_charger(world: &mut World, pos: Vec2) -> Entity {
    let mut charger = Charger::new(IVec2::ZERO);
    charger.amount = 25;
    charger.capacity = 100;
    world
        .spawn((
            charger,
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_loaded_hauler(world: &mut World, pos: Vec2) -> Entity {
    world
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            HaulerLoad {
                kind: ResourceKind::Minerals,
                amount: 30,
            },
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(32.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn assert_bar_overlays_exist(world: &mut World) {
    let targets = world.resource::<FillIndicatorTargets>().0.clone();
    let mut query = world.query::<&StructureOverlay>();
    let overlays: Vec<_> = query
        .iter(world)
        .filter(|overlay| targets.contains(&overlay.target))
        .collect();
    assert_eq!(overlays.len(), targets.len());
    assert!(
        overlays
            .iter()
            .any(|o| o.kind == StructureOverlayKind::Hauler)
    );

    let mut text_query = world.query::<(&StructureOverlay, Option<&Text2d>)>();
    for (overlay, text) in text_query.iter(world) {
        if targets.contains(&overlay.target) {
            assert!(text.is_none(), "fill overlay must not carry Text2d");
        }
    }
}
