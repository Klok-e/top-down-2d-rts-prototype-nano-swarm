//! Full-app evidence for loaded workers sharing a destination without remote claims.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::IntentGrid,
    nanobot::{
        ApproachPhase, Cargo, Commitment, Health, LogisticsReservation, Nanobot, NanobotType,
        OwnerSwarm, ReturningToStockpile, StrategicController, SwarmId, SwarmMember,
        VelocityComponent, WorkApproach,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
};

#[derive(Resource)]
struct Scene {
    workers: Vec<Entity>,
    stockpile: Entity,
    ticks: u32,
    captured_near: bool,
    captured_delivery: bool,
}

pub fn approach_delivery(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        prepare(ctx.world);
        return TestFlow::Screenshot("approach_delivery_before".into());
    }
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<Scene>();
    let stored = ctx.world.get::<Stockpile>(scene.stockpile).unwrap().amount;
    if scene.captured_delivery {
        assert_eq!(stored, 44, "delivery persists through screenshot readback");
        return TestFlow::Exit;
    }
    if !scene.captured_near
        && scene.workers.iter().any(|worker| {
            ctx.world
                .get::<Cargo>(*worker)
                .is_some_and(|cargo| cargo.amount > 0)
                && ctx
                    .world
                    .get::<WorkApproach>(*worker)
                    .is_some_and(|approach| {
                        matches!(
                            approach.phase,
                            ApproachPhase::Searching | ApproachPhase::Waiting
                        )
                    })
        })
    {
        ctx.world.resource_mut::<Scene>().captured_near = true;
        return TestFlow::Screenshot("approach_delivery_searching".into());
    }
    if stored == 44 {
        assert!(
            scene.captured_near,
            "capture loaded workers searching near the destination"
        );
        ctx.world.resource_mut::<Scene>().captured_delivery = true;
        return TestFlow::Screenshot("approach_delivery_all_delivered".into());
    }
    TestFlow::Continue
}

fn verify_tick(
    mut scene: ResMut<Scene>,
    stockpiles: Query<&Stockpile>,
    workers: Query<(), With<Nanobot>>,
) {
    scene.ticks += 1;
    for worker in &scene.workers {
        assert!(
            workers.contains(*worker),
            "all eleven workers remain in the rendered scene"
        );
    }
    let stored = stockpiles.get(scene.stockpile).unwrap().amount;
    assert!(
        scene.ticks <= 1200 || stored == 44,
        "delivery capture must become ready within twenty simulation seconds: stored={stored}"
    );
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
        .query_filtered::<Entity, With<StrategicController>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(entity).remove::<StrategicController>();
    }
    world.insert_resource(IntentGrid::new(4, 4));
    let cadence = std::time::Duration::from_secs_f64(1.0 / 60.0);
    world.insert_resource(Time::<Fixed>::from_duration(cadence));
    world.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(cadence));
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 500.0;
        transform.translation.y = 350.0;
        zoom.zoom = 1.8;
        if let Projection::Orthographic(projection) = &mut *projection {
            projection.scale = 1.8;
        }
    }
    let owner = world
        .query::<(Entity, &SwarmId)>()
        .iter(world)
        .find(|(_, swarm)| **swarm == SwarmId::PLAYER)
        .unwrap()
        .0;
    let stockpile = world
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 1000,
                radius: 32.0,
            },
            StockpileRole::Source,
            OwnerSwarm(owner),
            Sprite::from_color(Color::srgb(0.18, 0.42, 0.2), Vec2::splat(64.0)),
            Transform::from_xyz(700.0, 0.0, GAMEPLAY_SPRITE_Z),
        ))
        .id();
    let workers = (0..11)
        .map(|index| {
            let mut reservation = LogisticsReservation::new(
                Entity::PLACEHOLDER,
                stockpile,
                ResourceKind::Minerals,
                4,
            );
            reservation.source_remaining = 0;
            world
                .spawn((
                    Nanobot {},
                    NanobotType::Worker,
                    Commitment::Carrying,
                    VelocityComponent::default(),
                    Health::default(),
                    SwarmMember::new(SwarmId::PLAYER),
                    Cargo {
                        kind: ResourceKind::Minerals,
                        amount: 4,
                    },
                    ReturningToStockpile { stockpile },
                    reservation,
                    Transform::from_xyz(300.0, index as f32 * 75.0, GAMEPLAY_SPRITE_Z),
                ))
                .id()
        })
        .collect();
    let mut ledger = ResourceLedger::new();
    ledger.add_for(SwarmId::PLAYER, ResourceKind::Minerals, 44);
    world.insert_resource(ledger);
    world.insert_resource(Scene {
        workers,
        stockpile,
        ticks: 0,
        captured_near: false,
        captured_delivery: false,
    });
    world
        .resource_mut::<Schedules>()
        .get_mut(FixedPostUpdate)
        .unwrap()
        .add_systems(verify_tick);
}
