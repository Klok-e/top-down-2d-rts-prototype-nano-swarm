//! Visual evidence for live physical-custody and reservation indicators.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    nanobot::{
        Cargo, Charger, HaulerAssignment, Health, InteractionRegion, LogisticsReservation,
        MATERIAL_WORK_TICKS_PER_UNIT, MaterialWorkProgress, Nanobot, NanobotType, OwnerSwarm,
        ProductionFacility, Swarm, SwarmId, SwarmMember,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
    structure_overlay::{
        StructureOverlay, StructureOverlayKind, StructureOverlaySegment,
        StructureOverlaySegmentKind,
    },
    ui::match_banner::MatchBannerRoot,
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

#[derive(Resource)]
struct PhysicalLogisticsTargets {
    indicators: Vec<(Entity, StructureOverlayKind)>,
    delivery: Option<Delivery>,
}

#[derive(Clone, Copy)]
struct Delivery {
    pickup: Entity,
    facility: Entity,
    hauler: Entity,
}

pub fn physical_logistics(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        focus_camera(ctx.world);
        clear_nanobots_and_sprite_entities(ctx.world);
        despawn_match_banner(ctx.world);
        let targets = spawn_scene(ctx.world);
        ctx.world.insert_resource(targets);
        return TestFlow::Continue;
    }
    if ctx.frame < 12 {
        return TestFlow::Continue;
    }
    if ctx.frame == 12 {
        assert_scene_indicators(ctx.world);
        assert_match_banner_absent(ctx.world);
        return TestFlow::Screenshot("physical_logistics".to_string());
    }
    let Some(delivery) = ctx.world.resource::<PhysicalLogisticsTargets>().delivery else {
        return TestFlow::Exit;
    };
    if ctx.frame == 13 {
        begin_delivery_after_pickup_loss(ctx.world, delivery);
        return TestFlow::Continue;
    }
    let buffered = ctx
        .world
        .get::<ProductionFacility>(delivery.facility)
        .unwrap()
        .input_amount;
    let carried = ctx
        .world
        .get::<Cargo>(delivery.hauler)
        .map_or(0, |cargo| cargo.amount);
    assert_eq!(buffered + carried, 5);
    if carried > 0 {
        assert!(
            ctx.frame < 40,
            "the reserved delivery should finish after losing its pickup"
        );
        return TestFlow::Continue;
    }
    assert!(ctx.world.get_entity(delivery.pickup).is_err());
    ctx.world
        .resource_mut::<PhysicalLogisticsTargets>()
        .delivery = None;
    TestFlow::Screenshot("physical_logistics_delivered_after_pickup_loss".into())
}

fn begin_delivery_after_pickup_loss(world: &mut World, delivery: Delivery) {
    let owner = world
        .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
        .iter(world)
        .find(|(_, id)| **id == SwarmId::PLAYER)
        .unwrap()
        .0;
    world
        .entity_mut(delivery.facility)
        .insert(OwnerSwarm(owner));
    let facility = world.get::<Transform>(delivery.facility).unwrap();
    let current = world.get::<Transform>(delivery.hauler).unwrap();
    let position = InteractionRegion::structure(facility).approach(current.translation.truncate());
    let mut reservation = LogisticsReservation::new(
        delivery.pickup,
        delivery.facility,
        ResourceKind::Minerals,
        1,
    );
    reservation.source_remaining = 0;
    world.entity_mut(delivery.hauler).insert((
        Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 1,
        },
        HaulerAssignment {
            source: delivery.pickup,
            sink: delivery.facility,
            pickup_role: StockpileRole::Sink,
        },
        reservation,
        MaterialWorkProgress {
            ticks_elapsed: MATERIAL_WORK_TICKS_PER_UNIT - 1,
        },
    ));
    assert!(world.despawn(delivery.pickup));
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
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        zoom.zoom = 1.0;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.45;
        }
    }
}

fn spawn_scene(world: &mut World) -> PhysicalLogisticsTargets {
    let source = spawn_stockpile(world, Vec2::new(-150.0, -35.0), 16, StockpileRole::Source);
    let sink = spawn_stockpile(world, Vec2::new(-50.0, -35.0), 12, StockpileRole::Sink);
    let facility = spawn_facility(world, Vec2::new(50.0, -35.0));
    let charger = spawn_charger(world, Vec2::new(150.0, -35.0));
    let worker = spawn_cargo_bot(world, Vec2::new(-55.0, 70.0), NanobotType::Worker, 3);
    let hauler = spawn_cargo_bot(world, Vec2::new(55.0, 70.0), NanobotType::Hauler, 12);
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

    PhysicalLogisticsTargets {
        indicators: vec![
            (source, StructureOverlayKind::Stockpile),
            (sink, StructureOverlayKind::Stockpile),
            (facility, StructureOverlayKind::Facility),
            (charger, StructureOverlayKind::Charger),
            (worker, StructureOverlayKind::Worker),
            (hauler, StructureOverlayKind::Hauler),
        ],
        delivery: Some(Delivery {
            pickup: sink,
            facility,
            hauler,
        }),
    }
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

fn spawn_cargo_bot(world: &mut World, pos: Vec2, kind: NanobotType, amount: u32) -> Entity {
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
            Transform::from_translation(pos.extend(GAMEPLAY_SPRITE_Z)),
        ))
        .id()
}

fn assert_scene_indicators(world: &mut World) {
    let targets = world
        .resource::<PhysicalLogisticsTargets>()
        .indicators
        .clone();
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
