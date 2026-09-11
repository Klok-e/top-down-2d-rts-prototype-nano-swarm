//! Full-app movement evidence around completed structures and circular deposits.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::IntentGrid,
    nanobot::{
        Commitment, DirectMovementComponent, Health, Nanobot, NanobotType, StrategicController,
        Structure, StructureKind, SwarmId, SwarmMember, VelocityComponent,
    },
    resources::{ResourceDeposit, ResourceKind},
};

use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};

#[derive(Resource)]
struct NavigationScene {
    bots: Vec<(Entity, Vec2)>,
    started: bool,
    detour_captured: bool,
    arrived: bool,
}

pub fn shared_navigation(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        prepare(ctx.world);
        return TestFlow::Screenshot("shared_navigation_before".into());
    }
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<NavigationScene>();
    let bots = scene.bots.clone();
    let mut all_arrived = true;
    let mut detouring = false;
    for (bot, goal) in &bots {
        let position = ctx
            .world
            .get::<Transform>(*bot)
            .unwrap()
            .translation
            .truncate();
        for y in [72.0, 360.0, 648.0] {
            let rectangle_clearance = ((position - Vec2::new(360.0, y)).abs()
                - Vec2::new(36.0, 72.0))
            .max(Vec2::ZERO)
            .length();
            assert!(
                rectangle_clearance >= 33.99,
                "{bot:?} at {position:?} overlaps completed wall: {rectangle_clearance}"
            );
            let deposit_clearance = position.distance(Vec2::new(648.0, y)) - 48.0;
            assert!(
                deposit_clearance >= 33.99,
                "{bot:?} at {position:?} overlaps circular deposit: {deposit_clearance}"
            );
        }
        all_arrived &= position.distance(*goal) <= 2.01
            && ctx.world.get::<DirectMovementComponent>(*bot).is_none();
        detouring |= (300.0..=440.0).contains(&position.x) && (position.y - goal.y).abs() >= 100.0;
    }
    if scene.arrived {
        assert!(
            all_arrived,
            "arrived bots must remain at their final destinations during readback"
        );
        return TestFlow::Exit;
    }
    if !scene.started {
        for &(bot, goal) in &bots {
            ctx.world.entity_mut(bot).insert(DirectMovementComponent {
                xy: goal,
                stop_radius: 0.0,
                interaction: None,
                speed: None,
            });
        }
        ctx.world.resource_mut::<NavigationScene>().started = true;
    } else if !scene.detour_captured && detouring {
        ctx.world.resource_mut::<NavigationScene>().detour_captured = true;
        return TestFlow::Screenshot("shared_navigation_detour".into());
    } else if all_arrived {
        assert!(
            scene.detour_captured,
            "route must visibly detour around the completed walls"
        );
        ctx.world.resource_mut::<NavigationScene>().arrived = true;
        return TestFlow::Screenshot("shared_navigation_arrived".into());
    }
    assert!(
        ctx.frame < 1400,
        "all three bot types must reach their destinations: {:?}",
        bots.iter()
            .map(|(bot, _)| (
                ctx.world.get::<Transform>(*bot).unwrap().translation,
                ctx.world.get::<DirectMovementComponent>(*bot)
            ))
            .collect::<Vec<_>>()
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
        .query_filtered::<Entity, With<StrategicController>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        world.entity_mut(entity).remove::<StrategicController>();
    }
    world.insert_resource(IntentGrid::new(8, 8));
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 480.0;
        transform.translation.y = 360.0;
        zoom.zoom = 1.3;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 1.3;
        }
    }
    let mut bots = Vec::new();
    for (kind, name, y) in [
        (NanobotType::Worker, "WORKER", 72.0),
        (NanobotType::Hauler, "HAULER", 360.0),
        (NanobotType::Defender, "DEFENDER", 648.0),
    ] {
        world.spawn((
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.0)),
            Transform::from_xyz(360.0, y, GAMEPLAY_SPRITE_Z)
                .with_scale(Vec3::new(1.125, 2.25, 1.0)),
        ));
        let mesh = world.resource_mut::<Assets<Mesh>>().add(Circle::new(48.0));
        let material = world
            .resource_mut::<Assets<ColorMaterial>>()
            .add(Color::srgb(0.18, 0.65, 0.34));
        world.spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 80,
                capacity: 80,
                radius: 48.0,
            },
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform::from_xyz(648.0, y, GAMEPLAY_SPRITE_Z),
        ));
        world.spawn((
            Text2d::new(name),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            Transform::from_xyz(30.0, y + 65.0, GAMEPLAY_SPRITE_Z),
        ));
        world.spawn((
            Sprite::from_color(Color::srgb(0.2, 0.42, 0.6), Vec2::splat(8.0)),
            Transform::from_xyz(900.0, y, GAMEPLAY_SPRITE_Z),
        ));
        let bot = world
            .spawn((
                Nanobot {},
                kind,
                // An active travel task keeps idle Defender staging from replacing this goal.
                Commitment::Working,
                Health::default(),
                VelocityComponent::default(),
                SwarmMember(SwarmId::PLAYER),
                Transform::from_xyz(72.0, y, GAMEPLAY_SPRITE_Z + 1.0),
            ))
            .id();
        bots.push((bot, Vec2::new(900.0, y)));
    }
    world.insert_resource(NavigationScene {
        bots,
        started: false,
        detour_captured: false,
        arrived: false,
    });
}
