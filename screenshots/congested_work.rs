//! Full-app delivery uses free goal faces without displacing active workers.

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::IntentGrid,
    nanobot::{
        Cargo, Commitment, CongestionRecovery, DirectMovementComponent, HaulerAssignment, Health,
        LogisticsReservation, Nanobot, NanobotType, OpponentIntentController, OwnerSwarm,
        RemainingTravel, SwarmId, SwarmMember, VelocityComponent,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
};

#[derive(Resource)]
struct Scene {
    hauler: Entity,
    sink: Entity,
    workers: Vec<(Entity, Vec2)>,
    captured_delivery: bool,
}

pub fn congested_work(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        prepare(ctx.world);
        return TestFlow::Screenshot("congested_work_occupied_approach".into());
    }
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<Scene>();
    let stored = ctx.world.get::<Stockpile>(scene.sink).unwrap().amount;
    let carried = ctx
        .world
        .get::<Cargo>(scene.hauler)
        .map_or(0, |cargo| cargo.amount);
    assert_eq!(stored + carried, 20, "traffic must conserve cargo");
    let position = ctx
        .world
        .get::<Transform>(scene.hauler)
        .unwrap()
        .translation
        .truncate();
    for (worker, original) in &scene.workers {
        let current = ctx
            .world
            .get::<Transform>(*worker)
            .unwrap()
            .translation
            .truncate();
        assert!(
            current.distance(*original) < 0.001,
            "active workers keep their positions: worker {worker:?} original {original:?} current {current:?}, commitment {:?}",
            ctx.world.get::<Commitment>(*worker)
        );
        if stored > 0 {
            assert!(
                position.distance(current) >= 67.999,
                "unloading needs separate standing space"
            );
        }
    }
    if scene.captured_delivery {
        assert_eq!(
            stored, 20,
            "delivered cargo persists through image readback"
        );
        return TestFlow::Exit;
    }
    if stored == 20 {
        assert!(
            position.y.abs() > 32.0 || position.x > 400.0,
            "hauler must deliver from another goal face: {position:?}"
        );
        ctx.world.resource_mut::<Scene>().captured_delivery = true;
        return TestFlow::Screenshot("congested_work_delivered_other_face".into());
    }
    assert!(
        ctx.frame < 2000,
        "hauler must deliver around occupied goal approach; position {position:?}, cargo {carried}, movement {:?}, recovery {:?}, remaining {:?}",
        ctx.world.get::<DirectMovementComponent>(scene.hauler),
        ctx.world.get::<CongestionRecovery>(scene.hauler),
        ctx.world.get::<RemainingTravel>(scene.hauler)
    );
    TestFlow::Continue
}

fn prepare(world: &mut World) {
    clear_nanobots_and_sprite_entities(world);
    for entity in world
        .query_filtered::<Entity, Or<(With<Node>, With<Mesh2d>)>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        let _ = world.despawn(entity);
    }
    for entity in world
        .query_filtered::<Entity, With<OpponentIntentController>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world
            .entity_mut(entity)
            .remove::<OpponentIntentController>();
    }
    world.insert_resource(IntentGrid::new(4, 4));
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 200.0;
        transform.translation.y = 0.0;
        zoom.zoom = 0.65;
        if let Projection::Orthographic(projection) = &mut *projection {
            projection.scale = 0.65;
        }
    }
    let owner = world
        .query::<(Entity, &SwarmId)>()
        .iter(world)
        .find(|(_, swarm)| **swarm == SwarmId::PLAYER)
        .unwrap()
        .0;
    let mut stockpile = |x, capacity, role, color| {
        world
            .spawn((
                Stockpile {
                    kind: ResourceKind::Minerals,
                    amount: 0,
                    capacity,
                    radius: 32.0,
                },
                role,
                OwnerSwarm(owner),
                Sprite::from_color(color, Vec2::splat(64.0)),
                Transform::from_xyz(x, 0.0, GAMEPLAY_SPRITE_Z),
            ))
            .id()
    };
    let source = stockpile(
        -100.0,
        100,
        StockpileRole::Source,
        Color::srgb(0.18, 0.42, 0.20),
    );
    let sink = stockpile(
        400.0,
        100,
        StockpileRole::Sink,
        Color::srgb(0.18, 0.30, 0.62),
    );
    let mut workers = Vec::new();
    for y in [-68.0, 0.0, 68.0] {
        let position = Vec2::new(332.0, y);
        let worker = world
            .spawn((
                Nanobot {},
                NanobotType::Worker,
                Commitment::Working,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z)),
            ))
            .id();
        workers.push((worker, position));
    }
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 20);
    reservation.source_remaining = 0;
    let hauler = world
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            Commitment::Idle,
            VelocityComponent::default(),
            Health::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 20,
            },
            HaulerAssignment { source, sink },
            reservation,
            Transform::from_xyz(150.0, 0.0, GAMEPLAY_SPRITE_Z),
        ))
        .id();
    world.insert_resource(Scene {
        hauler,
        sink,
        workers,
        captured_delivery: false,
    });
}
