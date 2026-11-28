//! Visual evidence for live physical-custody and reservation indicators.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, Charger, HaulerAssignment, HaulerLoading, Health, InteractionRegion,
        LogisticsReservation, MaterialWorkProgress, Nanobot, NanobotType, OwnerSwarm,
        ProductionFacility, Swarm, SwarmId, SwarmMember,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
    structure_overlay::{
        StructureOverlay, StructureOverlayKind, StructureOverlaySegment,
        StructureOverlaySegmentKind,
    },
    tuning::Tuning,
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
        focus_camera(ctx.world, Vec2::ZERO, 0.45);
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
        .resource_mut::<IntentGrid>()
        .paint(IVec2::NEG_Y, IntentKind::Build, SwarmId::PLAYER);
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
            ticks_elapsed: Tuning::default().logistics.material_work_ticks - 1,
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

fn focus_camera(world: &mut World, center: Vec2, scale: f32) {
    let mut query = world.query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>();
    for (mut transform, mut projection, mut zoom) in query.iter_mut(world) {
        transform.translation.x = center.x;
        transform.translation.y = center.y;
        zoom.zoom = 1.0;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = scale;
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

#[derive(Resource)]
struct RestartScene {
    facility: Entity,
    fast_pickup: Entity,
    fast_hauler: Entity,
    slow_hauler: Entity,
    phase: RestartCapture,
}

#[derive(Clone, Copy, Debug)]
enum RestartCapture {
    AwaitingPickup,
    Carrying,
    Producing,
    Finished,
}

pub fn restart_refill(ctx: &mut TestContext) -> TestFlow {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::StrategicController;

    if ctx.frame == 2 {
        focus_camera(ctx.world, Vec2::new(0.0, 48.0), 0.62);
        clear_nanobots_and_sprite_entities(ctx.world);
        despawn_match_banner(ctx.world);
        let controllers = ctx
            .world
            .query_filtered::<Entity, With<StrategicController>>()
            .iter(ctx.world)
            .collect::<Vec<_>>();
        for controller in controllers {
            ctx.world
                .entity_mut(controller)
                .remove::<StrategicController>();
        }
        let mut grid = IntentGrid::new(16, 16);
        grid.paint(IVec2::ZERO, IntentKind::Build, SwarmId::PLAYER);
        for cell in [IVec2::NEG_X, IVec2::ZERO] {
            grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER);
        }
        ctx.world.insert_resource(grid);
        let scene = spawn_restart_scene(ctx.world);
        ctx.world.insert_resource(scene);
    }
    if ctx.frame <= 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 3 {
        let mut grid = ctx.world.resource_mut::<IntentGrid>();
        for x in -2..=2 {
            grid.paint(IVec2::new(x, 2), IntentKind::Defend, SwarmId::PLAYER);
        }
        return TestFlow::Continue;
    }

    let scene = ctx.world.resource::<RestartScene>();
    let facility = ctx.world.get::<ProductionFacility>(scene.facility).unwrap();
    let fast_claim = ctx.world.get::<LogisticsReservation>(scene.fast_hauler);
    let fast_cargo = ctx
        .world
        .get::<Cargo>(scene.fast_hauler)
        .map_or(0, |cargo| cargo.amount);
    let slow_cargo = ctx
        .world
        .get::<Cargo>(scene.slow_hauler)
        .map_or(0, |cargo| cargo.amount);
    let (phase, screenshot) = match scene.phase {
        RestartCapture::AwaitingPickup if fast_claim.is_some() => {
            let claim = fast_claim.unwrap();
            assert_eq!(claim.source, scene.fast_pickup);
            assert_eq!(claim.destination, scene.facility);
            assert_eq!(claim.amount, 3);
            assert_eq!(facility.input_amount, 2);
            assert_eq!(facility.current_target, None);
            (RestartCapture::Carrying, "restart_refill_short_reserved")
        }
        RestartCapture::Carrying
            if fast_cargo == 3 && ctx.world.get::<HaulerLoading>(scene.fast_hauler).is_none() =>
        {
            assert_eq!(facility.input_amount, 2);
            assert_eq!(
                ctx.world
                    .get::<Stockpile>(scene.fast_pickup)
                    .unwrap()
                    .amount,
                7
            );
            assert_eq!(slow_cargo, 5);
            (RestartCapture::Producing, "restart_refill_short_carrying")
        }
        RestartCapture::Producing
            if facility.current_target == Some(NanobotType::Defender)
                && facility.progress >= 24 =>
        {
            assert_eq!(fast_cargo, 0);
            assert_eq!(slow_cargo, 5);
            (RestartCapture::Finished, "restart_refill_short_producing")
        }
        RestartCapture::Finished => return TestFlow::Exit,
        _ => {
            assert!(
                ctx.frame < 300,
                "restart is stuck in {:?}: facility={facility:?}, fast={fast_claim:?}, slow cargo={slow_cargo}, demand={:?}, work={:?}",
                scene.phase,
                ctx.world.resource::<top_down_2d_rts_prototype_nano_swarm::nanobot::PopulationDemand>(),
                ctx.world.resource::<top_down_2d_rts_prototype_nano_swarm::nanobot::ActionableProjection>(),
            );
            return TestFlow::Continue;
        }
    };
    ctx.world.resource_mut::<RestartScene>().phase = phase;
    TestFlow::Screenshot(screenshot.into())
}

fn spawn_restart_scene(world: &mut World) -> RestartScene {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Commitment, PlannedKind, VelocityComponent},
        resources::ResourceLedger,
        structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
    };

    let owner = world
        .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
        .iter(world)
        .find(|(_, id)| **id == SwarmId::PLAYER)
        .unwrap()
        .0;
    let mut tuning = world.resource_mut::<Tuning>();
    tuning.production.cost_per_bot = 5;
    tuning.production.input_capacity = 10;
    tuning.production.ticks_per_bot = 120;
    tuning.logistics.hauler_carry_capacity = 10;
    tuning.logistics.hauler_transfer_per_pulse = 1;
    tuning.logistics.material_work_ticks = 18;
    let mut facility = ProductionFacility::with_tuning(&tuning);
    facility.input_amount = 2;
    let fast_pickup = spawn_stockpile(world, Vec2::new(-180.0, 36.0), 10, StockpileRole::Source);
    let full_pickup = spawn_stockpile(world, Vec2::new(252.0, 36.0), 10, StockpileRole::Source);
    let facility = world
        .spawn((
            facility,
            OwnerSwarm(owner),
            Transform::from_xyz(36.0, 36.0, GAMEPLAY_SPRITE_Z),
        ))
        .id();
    for (entity, kind) in [
        (facility, PlannedKind::ProductionFacility),
        (fast_pickup, PlannedKind::SourceStockpile),
        (full_pickup, PlannedKind::SourceStockpile),
    ] {
        let sprite = world
            .resource::<StructureSprites>()
            .sprite(kind, StructureVisualState::Completed);
        world.entity_mut(entity).insert((
            sprite,
            StructureVisual::completed(kind),
            OwnerSwarm(owner),
        ));
    }
    let hauler = |world: &mut World, position: Vec2| {
        world
            .spawn((
                Nanobot {},
                NanobotType::Hauler,
                Health::default(),
                Commitment::Idle,
                VelocityComponent::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
            ))
            .id()
    };
    let fast_hauler = hauler(world, Vec2::new(-144.0, -48.0));
    let slow_hauler = hauler(world, Vec2::new(1500.0, 200.0));
    let mut claim = LogisticsReservation::new(full_pickup, facility, ResourceKind::Minerals, 5);
    claim.source_remaining = 0;
    world.entity_mut(slow_hauler).insert((
        HaulerAssignment {
            source: full_pickup,
            sink: facility,
            pickup_role: StockpileRole::Source,
        },
        Cargo {
            kind: ResourceKind::Minerals,
            amount: 5,
        },
        claim,
    ));
    world
        .resource_mut::<ResourceLedger>()
        .add_for(SwarmId::PLAYER, ResourceKind::Minerals, 27);
    RestartScene {
        facility,
        fast_pickup,
        fast_hauler,
        slow_hauler,
        phase: RestartCapture::AwaitingPickup,
    }
}
