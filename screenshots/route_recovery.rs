//! Full-app cargo waiting and resumed physical delivery after access opens.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::IntentGrid,
    nanobot::{
        Cargo, Commitment, HaulerAssignment, Health, LogisticsReservation, Nanobot, NanobotType,
        OwnerSwarm, StrategicController, SwarmId, SwarmMember, VelocityComponent,
    },
    resources::{ResourceKind, Stockpile, StockpileRole},
};
#[derive(Resource)]
struct Scene {
    bot: Entity,
    sink: Entity,
    wall: Entity,
    phase: u8,
}

pub fn route_recovery(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        prepare(ctx.world);
    }
    if ctx.frame < 20 {
        return TestFlow::Continue;
    }
    let Scene {
        bot,
        sink,
        wall,
        phase,
    } = *ctx.world.resource::<Scene>();
    let carried = ctx.world.get::<Cargo>(bot).map_or(0, |c| c.amount);
    let stored = ctx.world.get::<Stockpile>(sink).unwrap().amount;
    assert_eq!(carried + stored, 12);
    match phase {
        0 => {
            assert_eq!(carried, 12);
            if ctx
                .world
                .get::<LogisticsReservation>(bot)
                .unwrap()
                .destination_remaining
                != 0
            {
                assert!(
                    ctx.frame < 1000,
                    "blocked destination must resolve within the navigation budget"
                );
                return TestFlow::Continue;
            }
            ctx.world.resource_mut::<Scene>().phase = 1;
            TestFlow::Screenshot("route_recovery_waiting".into())
        }
        1 => {
            assert_eq!(carried, 12);
            ctx.world.despawn(wall);
            ctx.world.resource_mut::<Scene>().phase = 2;
            TestFlow::Continue
        }
        2 if stored == 12 => {
            assert!(ctx.world.get::<Transform>(bot).unwrap().translation.x > 275.0);
            ctx.world.resource_mut::<Scene>().phase = 3;
            TestFlow::Screenshot("route_recovery_delivered".into())
        }
        3 => TestFlow::Exit,
        _ => {
            assert!(ctx.frame < 1000, "cargo must resume after removal");
            TestFlow::Continue
        }
    }
}

fn prepare(world: &mut World) {
    clear_nanobots_and_sprite_entities(world);
    for e in world
        .query_filtered::<Entity, Or<(With<Node>, With<Mesh2d>)>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        let _ = world.despawn(e);
    }
    for e in world
        .query_filtered::<Entity, With<StrategicController>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(e).remove::<StrategicController>();
    }
    world.insert_resource(IntentGrid::new(2, 2));
    for (mut t, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        t.translation.x = 0.0;
        t.translation.y = 0.0;
        zoom.zoom = 1.5;
        if let Projection::Orthographic(p) = &mut *projection {
            p.scale = 1.5;
        }
    }
    let owner = world
        .query::<(Entity, &SwarmId)>()
        .iter(world)
        .find(|(_, id)| **id == SwarmId::PLAYER)
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
        -350.0,
        0,
        StockpileRole::Source,
        Color::srgb(0.18, 0.42, 0.20),
    );
    let sink = stockpile(
        350.0,
        100,
        StockpileRole::Sink,
        Color::srgb(0.18, 0.30, 0.62),
    );
    let wall = stockpile(0.0, 0, StockpileRole::Source, Color::srgb(0.55, 0.35, 0.2));
    world.get_mut::<Transform>(wall).unwrap().scale.y = 40.0;
    let mut reservation = LogisticsReservation::new(source, sink, ResourceKind::Minerals, 12);
    reservation.source_remaining = 0;
    let bot = world
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            Commitment::Working,
            Health::default(),
            SwarmMember::new(SwarmId::PLAYER),
            VelocityComponent::default(),
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 12,
            },
            HaulerAssignment { source, sink },
            reservation,
            Transform::from_xyz(-180.0, 0.0, GAMEPLAY_SPRITE_Z),
        ))
        .id();
    world.insert_resource(Scene {
        bot,
        sink,
        wall,
        phase: 0,
    });
}
