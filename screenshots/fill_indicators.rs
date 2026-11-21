//! Screenshot evidence for bar-style structure and hauler fill indicators.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        Charger, HaulerLoad, MaintenanceProgress, Nanobot, NanobotType, OwnerSwarm, PlannedKind,
        PlannedStructure, ProductionFacility, SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure,
        StructureKind, Swarm, SwarmId, SwarmMember, completed_visual_color, planned_visual_color,
    },
    navigation::STRUCTURE_SPRITE_SIZE,
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
    structure_overlay::{
        ConditionOverlay, ConditionOverlayKind, StructureOverlay, StructureOverlayKind,
    },
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
    tuning::Tuning,
    ui::match_banner::MatchBannerRoot,
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

#[derive(Resource)]
struct FillIndicatorTargets {
    resource: Vec<Entity>,
    condition: Vec<Entity>,
}

pub fn fill_indicators(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        clear_nanobots_and_sprite_entities(ctx.world);
        despawn_match_banner(ctx.world);
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
        assert_match_banner_absent(ctx.world);
        return TestFlow::Screenshot("fill_indicators".to_string());
    }

    TestFlow::Exit
}

fn despawn_match_banner(world: &mut World) {
    let entities = world
        .query_filtered::<Entity, With<MatchBannerRoot>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        let _ = world.despawn(entity);
    }
}

fn assert_match_banner_absent(world: &mut World) {
    assert_eq!(
        world
            .query_filtered::<Entity, With<MatchBannerRoot>>()
            .iter(world)
            .count(),
        0,
    );
}

fn focus_camera(world: &mut World) {
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = 464.0;
        transform.translation.y = 590.0;
        zoom.zoom = 2.0;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 2.0;
        }
    }
}

fn spawn_focused_scene(world: &mut World) -> FillIndicatorTargets {
    let [player, opponent] = [true, false].map(|is_player| {
        let mut swarms = world.query_filtered::<(Entity, &SwarmId), With<Swarm>>();
        swarms
            .iter(world)
            .find(|(_, id)| id.is_player() == is_player)
            .unwrap()
            .0
    });
    let deposit = spawn_deposit(world, Vec2::new(40.0, 720.0));
    let stockpile = spawn_stockpile(world, Vec2::new(150.0, 720.0));
    let facility = spawn_facility(world, Vec2::new(300.0, 720.0));
    let planned = spawn_planned(world, Vec2::new(800.0, 720.0));
    let charger = spawn_charger(world, Vec2::new(645.0, 720.0));
    let match_charger = spawn_charger(world, Vec2::new(530.0, 720.0));
    let non_operational = spawn_non_operational_stockpile(world, Vec2::new(440.0, 720.0));
    let hauler = spawn_loaded_hauler(world, Vec2::new(930.0, 720.0));
    let seed_facility = spawn_facility(world, Vec2::new(150.0, 400.0));
    world.get_mut::<Transform>(seed_facility).unwrap().scale = Vec3::new(3.375, 3.375, 1.0);
    for (entity, kind, owner) in [
        (stockpile, PlannedKind::SourceStockpile, player),
        (facility, PlannedKind::ProductionFacility, player),
        (seed_facility, PlannedKind::ProductionFacility, player),
        (non_operational, PlannedKind::SinkStockpile, player),
        (charger, PlannedKind::Charger, opponent),
        (match_charger, PlannedKind::Charger, player),
    ] {
        insert_fixture_art(world, entity, StructureVisual::completed(kind), owner);
    }
    insert_fixture_art(
        world,
        planned,
        StructureVisual::planned(PlannedKind::Charger),
        opponent,
    );
    let maintenance_target = world
        .spawn((
            Structure::new(StructureKind::Basic),
            Transform::from_xyz(10_000.0, 10_000.0, GAMEPLAY_SPRITE_Z),
        ))
        .id();
    let worker = spawn_maintenance_worker(world, Vec2::new(330.0, 500.0), maintenance_target);
    FillIndicatorTargets {
        resource: vec![
            deposit,
            stockpile,
            facility,
            planned,
            charger,
            match_charger,
            non_operational,
            hauler,
            seed_facility,
        ],
        condition: vec![
            stockpile,
            facility,
            charger,
            match_charger,
            non_operational,
            seed_facility,
            worker,
        ],
    }
}

fn insert_fixture_art(world: &mut World, entity: Entity, visual: StructureVisual, owner: Entity) {
    let mut sprite = world
        .resource::<StructureSprites>()
        .sprite(visual.kind, visual.state);
    sprite.custom_size = Some(Vec2::splat(STRUCTURE_SPRITE_SIZE));
    sprite.color = match visual.state {
        StructureVisualState::Planned => planned_visual_color(),
        StructureVisualState::Completed => completed_visual_color(),
    };
    world
        .entity_mut(entity)
        .insert((OwnerSwarm(owner), sprite, visual));
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
        progress.ticks_worked = Tuning::default().maintenance.work_ticks / 2;
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
            StockpileRole::Source,
            fixture_transform(pos),
        ))
        .id()
}

fn spawn_facility(world: &mut World, pos: Vec2) -> Entity {
    let mut facility = ProductionFacility::new();
    facility.input_amount = 120;
    facility.input_capacity = 200;
    let mut condition = Structure::new(StructureKind::Basic);
    condition.ticks_since_maintained = Tuning::default().maintenance.needs_threshold;
    world
        .spawn((facility, condition, fixture_transform(pos)))
        .id()
}

fn spawn_planned(world: &mut World, pos: Vec2) -> Entity {
    let planned = PlannedStructure::new(PlannedKind::Charger, IVec2::ONE).with_work_remaining(750);
    world.spawn((planned, fixture_transform(pos))).id()
}

fn spawn_charger(world: &mut World, pos: Vec2) -> Entity {
    let mut charger = Charger::new(IVec2::ZERO);
    charger.amount = 25;
    charger.capacity = 100;
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = 60;
    world
        .spawn((charger, condition, fixture_transform(pos)))
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
            StockpileRole::Sink,
            fixture_transform(pos),
        ))
        .id()
}

fn fixture_transform(pos: Vec2) -> Transform {
    Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z))
        .with_scale(Vec3::new(1.125, 1.125, 1.0))
}

fn spawn_maintenance_worker(world: &mut World, pos: Vec2, target: Entity) -> Entity {
    world
        .spawn((
            Nanobot {},
            NanobotType::Worker,
            MaintenanceProgress {
                cell: IVec2::ZERO,
                target,
                ticks_worked: Tuning::default().maintenance.work_ticks / 2,
            },
            SwarmMember::new(SwarmId::PLAYER),
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
            SwarmMember::new(SwarmId::PLAYER),
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
    assert_eq!(condition_overlays.len(), 13);
    assert!(
        condition_overlays
            .iter()
            .any(|overlay| overlay.kind == ConditionOverlayKind::WorkerProgress)
    );
}
