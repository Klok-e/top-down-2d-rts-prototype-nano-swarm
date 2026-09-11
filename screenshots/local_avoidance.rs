//! Offscreen evidence of opposing streams clearing a one-cell bottleneck.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::IntentGrid,
    nanobot::{
        Commitment, CongestionRecovery, DirectMovementComponent, Health, Nanobot, NanobotType,
        StrategicController, Structure, StructureKind, SwarmId, SwarmMember, VelocityComponent,
    },
};

#[derive(Resource)]
struct TrafficScene {
    bots: Vec<(Entity, Vec2)>,
    started: bool,
    yielded: bool,
    arrived: bool,
}

pub fn local_avoidance(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare(ctx.world);
        return TestFlow::Screenshot("local_avoidance_before".into());
    }
    let scene = ctx.world.resource::<TrafficScene>();
    let bots = scene.bots.clone();
    let positions: Vec<_> = bots
        .iter()
        .map(|(entity, _)| {
            ctx.world
                .get::<Transform>(*entity)
                .unwrap()
                .translation
                .truncate()
        })
        .collect();
    for (index, position) in positions.iter().enumerate() {
        assert!(
            ctx.world
                .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
                .point_clear(*position)
        );
        for (other_index, other) in positions.iter().enumerate().skip(index + 1) {
            assert!(
                position.distance(*other) >= 67.99
                    || [bots[index].0, bots[other_index].0]
                        .into_iter()
                        .any(|bot| ctx.world.get::<CongestionRecovery>(bot).is_some()),
                "rendered bodies overlap outside recovery: {positions:?}"
            );
        }
    }
    // Arrived bodies may step aside for travellers still finishing their crossing.
    let arrived = scene.started
        && bots
            .iter()
            .all(|(bot, _)| ctx.world.get::<DirectMovementComponent>(*bot).is_none());
    if scene.arrived {
        assert!(arrived);
        return TestFlow::Exit;
    }
    if !scene.started {
        for &(entity, xy) in &bots {
            ctx.world
                .entity_mut(entity)
                .insert(DirectMovementComponent {
                    xy,
                    stop_radius: 0.,
                    interaction: None,
                    speed: None,
                });
        }
        ctx.world.resource_mut::<TrafficScene>().started = true;
    } else if !scene.yielded
        && positions
            .iter()
            .any(|position| (position.y - 324.).abs() > 50.)
    {
        ctx.world.resource_mut::<TrafficScene>().yielded = true;
        return TestFlow::Screenshot("local_avoidance_yield".into());
    } else if arrived {
        assert!(scene.yielded);
        ctx.world.resource_mut::<TrafficScene>().arrived = true;
        return TestFlow::Screenshot("local_avoidance_arrived".into());
    }
    assert!(
        ctx.frame < 2400,
        "traffic must resume retained routes: {positions:?}; orders={:?}; navigation={:?}",
        bots.iter()
            .map(|(entity, _)| ctx.world.get::<DirectMovementComponent>(*entity))
            .collect::<Vec<_>>(),
        ctx.world
            .resource::<top_down_2d_rts_prototype_nano_swarm::navigation::Navigation>()
            .work()
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
        transform.translation.x = 504.;
        transform.translation.y = 288.;
        zoom.zoom = 1.1;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 1.1;
        }
    }
    for y in [252., 396.] {
        world.spawn((
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.)),
            Transform::from_xyz(432., y, GAMEPLAY_SPRITE_Z).with_scale(Vec3::new(4.5, 1.125, 1.)),
        ));
    }
    world.spawn((
        Text2d::new("OPPOSING TRAFFIC / ONE-CELL PASSAGE"),
        TextFont {
            font_size: 22.,
            ..default()
        },
        Transform::from_xyz(504., 500., GAMEPLAY_SPRITE_Z),
    ));
    let mut bots = Vec::new();
    for (start, goal) in [(340., 828.), (412., 900.), (484., 180.), (556., 108.)] {
        let entity = world
            .spawn((
                Nanobot {},
                NanobotType::Worker,
                Commitment::Working,
                Health::default(),
                VelocityComponent::default(),
                SwarmMember(SwarmId::PLAYER),
                Transform::from_xyz(start, 324., GAMEPLAY_SPRITE_Z + 1.),
            ))
            .id();
        bots.push((entity, Vec2::new(goal, 324.)));
        world.spawn((
            Sprite::from_color(Color::srgb(0.2, 0.42, 0.6), Vec2::splat(8.)),
            Transform::from_xyz(goal, 324., GAMEPLAY_SPRITE_Z),
        ));
    }
    world.insert_resource(TrafficScene {
        bots,
        started: false,
        yielded: false,
        arrived: false,
    });
}

pub fn startup_formations(ctx: &mut TestContext) -> TestFlow {
    ctx.world.resource_mut::<Time<Virtual>>().pause();
    if ctx.frame == 0 {
        for (mut transform, mut projection, mut zoom) in ctx
            .world
            .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
            .iter_mut(ctx.world)
        {
            transform.translation.x = 256.;
            transform.translation.y = 256.;
            zoom.zoom = 1.8;
            if let Projection::Orthographic(ortho) = &mut *projection {
                ortho.scale = 1.8;
            }
        }
        for entity in ctx
            .world
            .query_filtered::<Entity, With<Node>>()
            .iter(ctx.world)
            .collect::<Vec<_>>()
        {
            ctx.world.entity_mut(entity).insert(Visibility::Hidden);
        }
    }
    let positions = ctx
        .world
        .query_filtered::<&Transform, With<Nanobot>>()
        .iter(ctx.world)
        .map(|transform| transform.translation.truncate())
        .collect::<Vec<_>>();
    assert_eq!(
        positions.len(),
        18,
        "authored seed counts must remain unchanged"
    );
    let shapes = ctx
        .world
        .query_filtered::<&Transform, With<Structure>>()
        .iter(ctx.world)
        .map(top_down_2d_rts_prototype_nano_swarm::navigation::Obstacle::structure)
        .collect::<Vec<_>>();
    for (index, position) in positions.iter().enumerate() {
        for other in &positions[index + 1..] {
            assert!(
                position.distance(*other) >= 67.99,
                "authored seed formation overlaps"
            );
        }
        assert!(
            shapes.iter().all(|shape| shape.admits_body(*position)),
            "authored seed starts inside a completed structure"
        );
    }
    if ctx.frame == 2 {
        return TestFlow::Screenshot("startup_formations".into());
    }
    if ctx.frame > 2 {
        TestFlow::Exit
    } else {
        TestFlow::Continue
    }
}
