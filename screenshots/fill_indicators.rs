//! Screenshot evidence for bar-style structure and hauler fill indicators.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        Charger, HaulerLoad, MAINTENANCE_NEEDS_THRESHOLD, MAINTENANCE_WORK_DURATION_TICKS,
        MaintenanceProgress, Nanobot, NanobotType, PlannedKind, PlannedStructure,
        ProductionFacility, SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
    structure_overlay::{
        ConditionOverlay, ConditionOverlayKind, StructureOverlay, StructureOverlayKind,
    },
};

use crate::harness::{TestContext, TestFlow};

#[derive(Resource)]
struct FillIndicatorTargets {
    resource: Vec<Entity>,
    condition: Vec<Entity>,
}

pub fn fill_indicators(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        despawn_existing_sprites(ctx.world);
        let targets = spawn_focused_scene(ctx.world);
        ctx.world.insert_resource(targets);
        return TestFlow::Continue;
    }

    if ctx.frame < 12 {
        keep_worker_progress_visible(ctx.world);
        return TestFlow::Continue;
    }

    if ctx.frame == 12 {
        keep_worker_progress_visible(ctx.world);
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

fn spawn_focused_scene(world: &mut World) -> FillIndicatorTargets {
    let deposit = spawn_deposit(world, Vec2::new(-270.0, -35.0));
    let stockpile = spawn_stockpile(world, Vec2::new(-180.0, -35.0));
    let facility = spawn_facility(world, Vec2::new(-90.0, -35.0));
    let planned = spawn_planned(world, Vec2::new(0.0, -35.0));
    let charger = spawn_charger(world, Vec2::new(90.0, -35.0));
    let non_operational = spawn_non_operational_stockpile(world, Vec2::new(180.0, -35.0));
    let hauler = spawn_loaded_hauler(world, Vec2::new(270.0, -35.0));
    let maintenance_target = world
        .spawn((
            Structure::new(StructureKind::Basic),
            Transform::from_xyz(10_000.0, 10_000.0, GAMEPLAY_SPRITE_Z),
        ))
        .id();
    let worker = spawn_maintenance_worker(world, Vec2::new(-90.0, -120.0), maintenance_target);
    FillIndicatorTargets {
        resource: vec![
            deposit,
            stockpile,
            facility,
            planned,
            charger,
            non_operational,
            hauler,
        ],
        condition: vec![stockpile, facility, charger, non_operational, worker],
    }
}

fn keep_worker_progress_visible(world: &mut World) {
    let Some(worker) = world
        .get_resource::<FillIndicatorTargets>()
        .and_then(|targets| targets.condition.last())
        .copied()
    else {
        return;
    };
    if let Some(mut progress) = world.entity_mut(worker).get_mut::<MaintenanceProgress>() {
        progress.ticks_worked = MAINTENANCE_WORK_DURATION_TICKS / 2;
    }
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
            Structure::new(StructureKind::Basic),
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
    let mut condition = Structure::new(StructureKind::Basic);
    condition.ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
    world
        .spawn((
            facility,
            condition,
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
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = 60;
    world
        .spawn((
            charger,
            condition,
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_non_operational_stockpile(world: &mut World, pos: Vec2) -> Entity {
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    world
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 200,
                capacity: 1000,
                radius: 32.0,
            },
            condition,
            Sprite {
                color: Color::srgb(0.22, 0.22, 0.22),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_maintenance_worker(world: &mut World, pos: Vec2, target: Entity) -> Entity {
    world
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            MaintenanceProgress {
                cell: IVec2::ZERO,
                target,
                ticks_worked: MAINTENANCE_WORK_DURATION_TICKS / 2,
            },
            Sprite {
                color: Color::srgb(0.25, 0.85, 0.35),
                custom_size: Some(Vec2::splat(32.0)),
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
    let resource_targets = world.resource::<FillIndicatorTargets>().resource.clone();
    let condition_targets = world.resource::<FillIndicatorTargets>().condition.clone();
    let mut query = world.query::<&StructureOverlay>();
    let overlays: Vec<_> = query
        .iter(world)
        .filter(|overlay| resource_targets.contains(&overlay.target))
        .collect();
    assert_eq!(overlays.len(), resource_targets.len());
    assert!(
        overlays
            .iter()
            .any(|o| o.kind == StructureOverlayKind::Hauler)
    );

    let mut text_query = world.query::<(&StructureOverlay, Option<&Text2d>)>();
    for (overlay, text) in text_query.iter(world) {
        if resource_targets.contains(&overlay.target) {
            assert!(text.is_none(), "fill overlay must not carry Text2d");
        }
    }

    let mut condition_query = world.query::<&ConditionOverlay>();
    let condition_overlays: Vec<_> = condition_query
        .iter(world)
        .filter(|overlay| condition_targets.contains(&overlay.target))
        .collect();
    assert_eq!(condition_overlays.len(), 9);
    assert!(
        condition_overlays
            .iter()
            .any(|overlay| overlay.kind == ConditionOverlayKind::WorkerProgress)
    );
}
