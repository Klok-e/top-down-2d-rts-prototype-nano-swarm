//! Visual evidence for live physical-custody and reservation indicators.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        Cargo, Charger, Health, LogisticsReservation, Nanobot, NanobotType, ProductionFacility,
        SwarmId, SwarmMember,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
    structure_overlay::{
        StructureOverlay, StructureOverlayKind, StructureOverlaySegment,
        StructureOverlaySegmentKind,
    },
};

use crate::harness::{TestContext, TestFlow};

#[derive(Resource)]
struct PhysicalLogisticsTargets(Vec<(Entity, StructureOverlayKind)>);

pub fn physical_logistics(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        despawn_existing_sprites(ctx.world);
        let targets = spawn_scene(ctx.world);
        ctx.world.insert_resource(PhysicalLogisticsTargets(targets));
        return TestFlow::Continue;
    }
    if ctx.frame < 12 {
        return TestFlow::Continue;
    }
    if ctx.frame == 12 {
        assert_scene_indicators(ctx.world);
        return TestFlow::Screenshot("physical_logistics".to_string());
    }
    TestFlow::Exit
}

fn focus_camera(world: &mut World) {
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        zoom.zoom = 1.0;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.45;
        }
    }
}

fn despawn_existing_sprites(world: &mut World) {
    let entities: Vec<_> = world
        .query_filtered::<Entity, With<Sprite>>()
        .iter(world)
        .collect();
    for entity in entities {
        let _ = world.despawn(entity);
    }
}

fn spawn_scene(world: &mut World) -> Vec<(Entity, StructureOverlayKind)> {
    let source = spawn_stockpile(world, Vec2::new(-150.0, -35.0), 16, StockpileRole::Source);
    let sink = spawn_stockpile(world, Vec2::new(-50.0, -35.0), 12, StockpileRole::Sink);
    let facility = spawn_facility(world, Vec2::new(50.0, -35.0));
    let charger = spawn_charger(world, Vec2::new(150.0, -35.0));
    let worker = spawn_cargo_bot(
        world,
        Vec2::new(-55.0, 70.0),
        NanobotType::Worker,
        3,
        Color::srgb(0.35, 0.95, 0.55),
    );
    let hauler = spawn_cargo_bot(
        world,
        Vec2::new(55.0, 70.0),
        NanobotType::Hauler,
        12,
        Color::srgb(0.20, 0.75, 0.95),
    );
    world.entity_mut(hauler).insert(LogisticsReservation {
        source,
        destination: facility,
        kind: ResourceKind::Minerals,
        amount: 12,
        source_remaining: 12,
        destination_remaining: 12,
    });
    world.spawn(LogisticsReservation {
        source: sink,
        destination: charger,
        kind: ResourceKind::Minerals,
        amount: 8,
        source_remaining: 8,
        destination_remaining: 8,
    });

    vec![
        (source, StructureOverlayKind::Stockpile),
        (sink, StructureOverlayKind::Stockpile),
        (facility, StructureOverlayKind::Facility),
        (charger, StructureOverlayKind::Charger),
        (worker, StructureOverlayKind::Worker),
        (hauler, StructureOverlayKind::Hauler),
    ]
}

fn spawn_stockpile(world: &mut World, pos: Vec2, amount: u32, role: StockpileRole) -> Entity {
    world
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount,
                capacity: 20,
                radius: 32.0,
            },
            role,
            Sprite {
                color: match role {
                    StockpileRole::Source => Color::srgb(0.18, 0.42, 0.20),
                    StockpileRole::Sink => Color::srgb(0.18, 0.30, 0.42),
                },
                custom_size: Some(Vec2::splat(58.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_facility(world: &mut World, pos: Vec2) -> Entity {
    let mut facility = ProductionFacility::new();
    facility.input_amount = 4;
    facility.input_capacity = 20;
    world
        .spawn((
            facility,
            Sprite {
                color: Color::srgb(0.18, 0.28, 0.55),
                custom_size: Some(Vec2::splat(58.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_charger(world: &mut World, pos: Vec2) -> Entity {
    let mut charger = Charger::new(IVec2::ZERO);
    charger.amount = 5;
    charger.capacity = 20;
    world
        .spawn((
            charger,
            Sprite {
                color: Color::srgb(0.42, 0.18, 0.55),
                custom_size: Some(Vec2::splat(58.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn spawn_cargo_bot(
    world: &mut World,
    pos: Vec2,
    kind: NanobotType,
    amount: u32,
    color: Color,
) -> Entity {
    world
        .spawn((
            Nanobot {},
            kind,
            Health::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Cargo {
                kind: ResourceKind::Minerals,
                amount,
            },
            Sprite {
                color,
                custom_size: Some(Vec2::splat(24.0)),
                ..default()
            },
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn assert_scene_indicators(world: &mut World) {
    let targets = world.resource::<PhysicalLogisticsTargets>().0.clone();
    let overlays: Vec<_> = world
        .query::<&StructureOverlay>()
        .iter(world)
        .filter(|overlay| targets.iter().any(|(target, _)| *target == overlay.target))
        .map(|overlay| {
            (
                overlay.target,
                overlay.kind,
                overlay.outgoing_reserved,
                overlay.incoming_reserved,
            )
        })
        .collect();
    assert_eq!(overlays.len(), targets.len());
    for expected in targets {
        assert!(
            overlays
                .iter()
                .any(|(target, kind, _, _)| (*target, *kind) == expected)
        );
    }
    for (_, kind, outgoing, incoming) in overlays {
        let reservation_entity = match kind {
            StructureOverlayKind::Stockpile => outgoing,
            StructureOverlayKind::Facility | StructureOverlayKind::Charger => incoming,
            _ => continue,
        };
        let entity = world.entity(reservation_entity);
        assert!(
            entity
                .get::<StructureOverlaySegment>()
                .is_some_and(|segment| segment.kind != StructureOverlaySegmentKind::Physical)
        );
        assert!(entity.get::<Sprite>().unwrap().custom_size.unwrap().x > 0.0);
    }
}
