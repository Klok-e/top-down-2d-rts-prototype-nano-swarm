//! Live gathering evidence at circular and scaled rectangular target surfaces.

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, Charger, Commitment, Health, Nanobot, NanobotType, OwnerSwarm, ProductionFacility,
        Structure, StructureKind, Swarm, SwarmId, SwarmMember, VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
};

#[derive(Resource)]
struct ExteriorScene {
    deposit: Entity,
    stockpile: Entity,
    worker: Entity,
    extracted: bool,
    delivered: bool,
}

pub fn exterior_work(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        clear_nanobots_and_sprite_entities(ctx.world);
        let nodes = ctx
            .world
            .query_filtered::<Entity, Or<(With<Node>, With<Mesh2d>)>>()
            .iter(ctx.world)
            .collect::<Vec<_>>();
        for entity in nodes {
            let _ = ctx.world.despawn(entity);
        }
        ctx.world.insert_resource(IntentGrid::new(8, 8));
        ctx.world
            .resource_mut::<IntentGrid>()
            .paint(IVec2::ZERO, IntentKind::Gather);
        for (mut transform, mut projection, mut zoom) in ctx
            .world
            .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
            .iter_mut(ctx.world)
        {
            transform.translation.x = 240.0;
            transform.translation.y = 140.0;
            zoom.zoom = 0.5;
            if let Projection::Orthographic(ortho) = &mut *projection {
                ortho.scale = 0.5;
            }
        }
        let mesh = ctx
            .world
            .resource_mut::<Assets<Mesh>>()
            .add(Circle::new(48.0));
        let material = ctx
            .world
            .resource_mut::<Assets<ColorMaterial>>()
            .add(Color::srgb(0.18, 0.65, 0.34));
        let deposit = ctx
            .world
            .spawn((
                ResourceDeposit {
                    kind: ResourceKind::Minerals,
                    amount: 80,
                    capacity: 80,
                    radius: 48.0,
                },
                Mesh2d(mesh),
                MeshMaterial2d(material),
                Transform::from_xyz(140.0, 140.0, GAMEPLAY_SPRITE_Z),
            ))
            .id();
        let stockpile = ctx
            .world
            .spawn((
                Stockpile {
                    kind: ResourceKind::Minerals,
                    amount: 0,
                    capacity: 100,
                    radius: 32.0,
                },
                StockpileRole::Source,
                Sprite::from_color(Color::srgb(0.18, 0.35, 0.62), Vec2::splat(64.0)),
                Transform::from_xyz(350.0, 140.0, GAMEPLAY_SPRITE_Z)
                    .with_scale(Vec3::new(2.0, 1.0, 1.0)),
            ))
            .id();
        let worker = ctx
            .world
            .spawn((
                Nanobot {},
                NanobotType::Worker,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_xyz(30.0, 140.0, GAMEPLAY_SPRITE_Z),
            ))
            .id();
        ctx.world.insert_resource(ExteriorScene {
            deposit,
            stockpile,
            worker,
            extracted: false,
            delivered: false,
        });
    }
    if ctx.frame <= 2 {
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<ExteriorScene>();
    if scene.delivered {
        return TestFlow::Exit;
    }
    let position = ctx
        .world
        .get::<Transform>(scene.worker)
        .unwrap()
        .translation
        .truncate();
    let amount = ctx
        .world
        .get::<ResourceDeposit>(scene.deposit)
        .unwrap()
        .amount;
    let stock = ctx.world.get::<Stockpile>(scene.stockpile).unwrap().amount;
    if !scene.extracted && amount < 80 {
        let clearance = position.distance(Vec2::new(140.0, 140.0)) - 48.0;
        assert!(
            (33.999..=38.001).contains(&clearance),
            "live extraction body clearance: {clearance}"
        );
        ctx.world.resource_mut::<ExteriorScene>().extracted = true;
        return TestFlow::Screenshot("exterior_work_gather".into());
    }
    if scene.extracted && stock > 0 {
        let clearance = ((position - Vec2::new(350.0, 140.0)).abs() - Vec2::new(64.0, 32.0))
            .max(Vec2::ZERO)
            .length();
        assert!(
            (33.999..=38.001).contains(&clearance),
            "live delivery body clearance: {clearance}"
        );
        ctx.world.resource_mut::<ExteriorScene>().delivered = true;
        return TestFlow::Screenshot("exterior_work_delivery".into());
    }
    assert!(
        ctx.frame < 500,
        "worker must gather and deliver through full app schedules"
    );
    TestFlow::Continue
}

#[derive(Resource)]
struct ServiceScene {
    target: Entity,
    bot: Entity,
    captured: bool,
}

fn prepare_service_scene(world: &mut World) -> Entity {
    clear_nanobots_and_sprite_entities(world);
    let entities = world
        .query_filtered::<Entity, Or<(With<Node>, With<Mesh2d>)>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        let _ = world.despawn(entity);
    }
    world.insert_resource(IntentGrid::new(8, 8));
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 240.0;
        transform.translation.y = 140.0;
        zoom.zoom = 0.5;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.5;
        }
    }
    world
        .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
        .iter(world)
        .find(|(_, id)| **id == SwarmId::PLAYER)
        .expect("startup player swarm")
        .0
}

pub fn exterior_hauler_delivery(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        let owner = prepare_service_scene(ctx.world);
        ctx.world.spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 20,
                capacity: 100,
                radius: 32.0,
            },
            StockpileRole::Sink,
            OwnerSwarm(owner),
            Sprite::from_color(Color::srgb(0.18, 0.35, 0.62), Vec2::splat(64.0)),
            Transform::from_xyz(140.0, 140.0, GAMEPLAY_SPRITE_Z)
                .with_scale(Vec3::new(2.0, 1.0, 1.0)),
        ));
        let target = ctx
            .world
            .spawn((
                ProductionFacility::new(),
                OwnerSwarm(owner),
                Sprite::from_color(Color::srgb(0.65, 0.35, 0.18), Vec2::splat(64.0)),
                Transform::from_xyz(350.0, 140.0, GAMEPLAY_SPRITE_Z),
            ))
            .id();
        let bot = ctx
            .world
            .spawn((
                Nanobot {},
                NanobotType::Hauler,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_xyz(30.0, 140.0, GAMEPLAY_SPRITE_Z),
            ))
            .id();
        ctx.world.insert_resource(ServiceScene {
            target,
            bot,
            captured: false,
        });
    }
    if ctx.frame <= 2 {
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<ServiceScene>();
    if scene.captured {
        return TestFlow::Exit;
    }
    if ctx
        .world
        .get::<ProductionFacility>(scene.target)
        .unwrap()
        .input_amount
        > 0
    {
        let position = ctx
            .world
            .get::<Transform>(scene.bot)
            .unwrap()
            .translation
            .truncate();
        let clearance = ((position - Vec2::new(350.0, 140.0)).abs() - Vec2::splat(32.0))
            .max(Vec2::ZERO)
            .length();
        assert!(
            (33.999..=38.001).contains(&clearance),
            "live Hauler unloading clearance: {clearance}"
        );
        ctx.world.resource_mut::<ServiceScene>().captured = true;
        return TestFlow::Screenshot("exterior_hauler_delivery".into());
    }
    assert!(
        ctx.frame < 500,
        "Hauler must deliver a physical load to the terminal"
    );
    TestFlow::Continue
}

pub fn exterior_defender_charging(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        let owner = prepare_service_scene(ctx.world);
        ctx.world.resource_mut::<IntentGrid>().paint_owned(
            IVec2::ZERO,
            IntentKind::Defend,
            Some(SwarmId::PLAYER),
        );
        let mut charger = Charger::new(IVec2::ZERO);
        charger.amount = 40;
        let target = ctx
            .world
            .spawn((
                charger,
                Structure::new(StructureKind::Basic),
                OwnerSwarm(owner),
                Sprite::from_color(Color::srgb(0.55, 0.25, 0.65), Vec2::splat(64.0)),
                Transform::from_xyz(300.0, 140.0, GAMEPLAY_SPRITE_Z)
                    .with_scale(Vec3::new(2.0, 1.0, 1.0)),
            ))
            .id();
        let bot = ctx
            .world
            .spawn((
                Nanobot {},
                NanobotType::Defender,
                Commitment::Idle,
                VelocityComponent::default(),
                Health::default(),
                SwarmMember::new(SwarmId::PLAYER),
                Transform::from_xyz(140.0, 140.0, GAMEPLAY_SPRITE_Z),
            ))
            .id();
        ctx.world.entity_mut(bot).insert(Charge {
            current: 0.1,
            max: 1.0,
        });
        ctx.world.insert_resource(ServiceScene {
            target,
            bot,
            captured: false,
        });
    }
    if ctx.frame <= 2 {
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<ServiceScene>();
    if scene.captured {
        return TestFlow::Exit;
    }
    if ctx.world.get::<Charger>(scene.target).unwrap().amount < 40 {
        assert!(
            ctx.world.get::<Charge>(scene.bot).unwrap().current > 0.1,
            "supplied pulse restores Defender Charge"
        );
        let position = ctx
            .world
            .get::<Transform>(scene.bot)
            .unwrap()
            .translation
            .truncate();
        let clearance = ((position - Vec2::new(300.0, 140.0)).abs() - Vec2::new(64.0, 32.0))
            .max(Vec2::ZERO)
            .length();
        assert!(
            (33.999..=38.001).contains(&clearance),
            "live Defender charging clearance: {clearance}"
        );
        ctx.world.resource_mut::<ServiceScene>().captured = true;
        return TestFlow::Screenshot("exterior_defender_charging".into());
    }
    assert!(
        ctx.frame < 200,
        "Defender must reach Charger and receive a supplied pulse"
    );
    TestFlow::Continue
}
