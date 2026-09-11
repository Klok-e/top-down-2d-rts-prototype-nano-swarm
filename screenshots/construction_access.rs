//! A planned Charger leaves the only friendly passage open.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Charge, NanobotBundle, NanobotType, OwnerSwarm, PlannedKind, PlannedStructure,
        StrategicController, Swarm, SwarmId,
    },
    resources::{ResourceKind, Stockpile},
};

#[derive(Resource)]
struct AccessScene {
    painted: bool,
    captured: bool,
}

pub fn construction_access(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        prepare(ctx.world);
        return TestFlow::Screenshot("construction_access_open_passage".into());
    }
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    for mut charge in ctx.world.query::<&mut Charge>().iter_mut(ctx.world) {
        charge.current = 0.1;
    }
    if !ctx.world.resource::<AccessScene>().painted {
        ctx.world.resource_mut::<IntentGrid>().paint(
            IVec2::ZERO,
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
        ctx.world.resource_mut::<AccessScene>().painted = true;
    }
    let mut found = false;
    for (mut plan, transform) in ctx
        .world
        .query::<(&mut PlannedStructure, &Transform)>()
        .iter_mut(ctx.world)
    {
        if plan.kind != PlannedKind::Charger {
            continue;
        }
        found = true;
        assert!(
            transform
                .translation
                .truncate()
                .distance(Vec2::new(252.0, 252.0))
                >= 144.0,
            "Charger must leave the narrow connection clear"
        );
        *plan = plan.with_work_remaining(100_000);
    }
    if found {
        if ctx.world.resource::<AccessScene>().captured {
            return TestFlow::Exit;
        }
        ctx.world.resource_mut::<AccessScene>().captured = true;
        return TestFlow::Screenshot("construction_access_safe_alternative".into());
    }
    assert!(
        ctx.frame < 150,
        "service demand must select a safe alternate site: {:?}",
        ctx.world
            .query::<(&NanobotType, &Transform, Option<&Charge>)>()
            .iter(ctx.world)
            .map(|(k, t, c)| (*k, t.translation, c.map(|c| c.current)))
            .collect::<Vec<_>>()
    );
    TestFlow::Continue
}

fn prepare(world: &mut World) {
    clear_nanobots_and_sprite_entities(world);
    for entity in world
        .query_filtered::<Entity, Or<(
            With<Mesh2d>,
            With<Node>,
            With<Swarm>,
            With<StrategicController>,
        )>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        let _ = world.despawn(entity);
    }
    world.insert_resource(IntentGrid::new(2, 2));
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 252.0;
        transform.translation.y = 200.0;
        zoom.zoom = 0.9;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.9;
        }
    }
    let friendly = world
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()))
        .id();
    let enemy = world
        .spawn((Swarm {}, SwarmId(2), Transform::default()))
        .id();
    for (center, half, owner, color) in [
        (
            Vec2::new(252.0, -166.0),
            Vec2::new(36.0, 346.0),
            enemy,
            Color::srgb(0.65, 0.25, 0.18),
        ),
        (
            Vec2::new(252.0, 418.0),
            Vec2::new(36.0, 94.0),
            enemy,
            Color::srgb(0.65, 0.25, 0.18),
        ),
        (
            Vec2::new(36.0, 252.0),
            Vec2::splat(36.0),
            friendly,
            Color::srgb(0.2, 0.6, 0.3),
        ),
        (
            Vec2::new(468.0, 252.0),
            Vec2::splat(36.0),
            friendly,
            Color::srgb(0.2, 0.6, 0.3),
        ),
    ] {
        world.spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 20,
                radius: 32.0,
            },
            OwnerSwarm(owner),
            Sprite::from_color(color, Vec2::splat(64.0)),
            Transform::from_translation(center.extend(GAMEPLAY_SPRITE_Z))
                .with_scale((half / 32.0).extend(1.0)),
        ));
    }
    world.spawn((
        NanobotBundle::default(),
        Transform::from_xyz(108.0, 108.0, GAMEPLAY_SPRITE_Z),
        Sprite::from_color(Color::WHITE, Vec2::splat(48.0)),
    ));
    world.spawn((
        NanobotBundle {
            nanobot_type: NanobotType::Defender,
            ..Default::default()
        },
        Charge {
            current: 0.1,
            ..Default::default()
        },
        Transform::from_xyz(252.0, 252.0, GAMEPLAY_SPRITE_Z),
        Sprite::from_color(Color::srgb(0.2, 0.7, 0.9), Vec2::splat(48.0)),
    ));
    world.insert_resource(AccessScene {
        painted: false,
        captured: false,
    });
}
