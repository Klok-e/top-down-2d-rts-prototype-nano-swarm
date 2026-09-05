//! Full-app evidence that completed output waits for traffic to clear its exit.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    game_settings::GameSettings,
    intent::IntentGrid,
    nanobot::{
        Commitment, DirectMovementComponent, Health, Nanobot, NanobotType,
        OpponentIntentController, PRODUCTION_TICKS_PER_BOT, PlannedKind, ProductionFacility,
        SwarmId, SwarmMember, VelocityComponent,
    },
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
};

#[derive(Resource)]
struct ExitScene {
    facility: Entity,
    blocker: Entity,
    caption: Entity,
    phase: u8,
    released_frame: u32,
}

pub fn production_exit(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare(ctx.world);
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<ExitScene>();
    let (facility, blocker, caption, phase, released_frame) = (
        scene.facility,
        scene.blocker,
        scene.caption,
        scene.phase,
        scene.released_frame,
    );
    let bots: Vec<_> = ctx
        .world
        .query_filtered::<(&NanobotType, &Transform), With<Nanobot>>()
        .iter(ctx.world)
        .map(|(kind, transform)| (*kind, transform.translation.truncate()))
        .collect();
    for (index, (_, position)) in bots.iter().enumerate() {
        for (_, other) in &bots[index + 1..] {
            assert!(
                position.distance(*other) >= 67.99,
                "output and traffic overlap: {bots:?}"
            );
        }
        assert!(
            (position.x - 252.).abs() >= 69.99 || (position.y - 252.).abs() >= 69.99,
            "a body occupies the facility: {position:?}"
        );
    }
    let production = ctx.world.get::<ProductionFacility>(facility).unwrap();
    if phase <= 1 {
        assert_eq!(bots.len(), 8, "blocked facility must retain its output");
        assert_eq!(production.current_target, Some(NanobotType::Hauler));
        assert_eq!(production.progress, PRODUCTION_TICKS_PER_BOT);
        assert_eq!(production.input_amount, 0);
        if phase == 0 && ctx.frame >= 24 {
            ctx.world.resource_mut::<ExitScene>().phase = 1;
            return TestFlow::Screenshot("production_exit_waiting".into());
        }
        if phase == 1 {
            ctx.world.resource_mut::<GameSettings>().bot_speed = 5.;
            // The screenshot pump advances simulation; recheck waiting above before moving.
            ctx.world
                .entity_mut(blocker)
                .insert(DirectMovementComponent {
                    xy: Vec2::new(252., 36.),
                    stop_radius: 0.,
                    interaction: None,
                    speed: Some(5.),
                });
            ctx.world.resource_mut::<ExitScene>().phase = 2;
        }
    } else if phase == 2 && production.current_target.is_none() {
        assert_eq!(bots.len(), 9, "opening one exit must release one output");
        let (_, output) = bots
            .iter()
            .find(|(kind, _)| *kind == NanobotType::Hauler)
            .unwrap();
        assert!(
            output.distance(Vec2::new(252., 180.)) < 0.01,
            "release must use the newly cleared exit: {output:?}"
        );
        let blocker_position = ctx
            .world
            .get::<Transform>(blocker)
            .unwrap()
            .translation
            .truncate();
        assert!(
            blocker_position.y <= 112.01,
            "traffic must physically move before release"
        );
        ctx.world.resource_mut::<GameSettings>().bot_speed = 0.;
        ctx.world.get_mut::<Text2d>(caption).unwrap().0 = "EXIT CLEAR / ONE HAULER RELEASED".into();
        let mut scene = ctx.world.resource_mut::<ExitScene>();
        scene.phase = 3;
        scene.released_frame = ctx.frame;
    } else if phase >= 3 {
        assert_eq!(bots.len(), 9, "finished cycle must not release twice");
        assert_eq!(
            bots.iter()
                .filter(|(kind, _)| *kind == NanobotType::Hauler)
                .count(),
            1
        );
        assert_eq!(production.current_target, None);
        assert_eq!(production.progress, 0);
        assert_eq!(production.input_amount, 0);
        if phase == 3 && ctx.frame >= released_frame + 30 {
            ctx.world.resource_mut::<ExitScene>().phase = 4;
            return TestFlow::Screenshot("production_exit_released".into());
        }
        if phase == 4 {
            return TestFlow::Exit;
        }
    }
    assert!(
        ctx.frame < 300,
        "finished output failed to leave the facility"
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
    world.insert_resource(IntentGrid::new(8, 8));
    world.resource_mut::<GameSettings>().bot_speed = 0.;
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 252.;
        transform.translation.y = 216.;
        zoom.zoom = 0.6;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.6;
        }
    }
    let mut production = ProductionFacility::new();
    production.current_target = Some(NanobotType::Hauler);
    production.progress = PRODUCTION_TICKS_PER_BOT;
    production.input_amount = 0;
    let kind = PlannedKind::ProductionFacility;
    let mut sprite = world
        .resource::<StructureSprites>()
        .sprite(kind, StructureVisualState::Completed);
    sprite.custom_size = Some(Vec2::splat(64.));
    let facility = world
        .spawn((
            production,
            sprite,
            StructureVisual::completed(kind),
            Transform::from_xyz(252., 252., GAMEPLAY_SPRITE_Z)
                .with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let mut blocker = None;
    for y in -1..=1 {
        for x in -1..=1 {
            if x == 0 && y == 0 {
                continue;
            }
            let bot = world
                .spawn((
                    Nanobot {},
                    NanobotType::Worker,
                    Commitment::Working,
                    Health::default(),
                    VelocityComponent::default(),
                    SwarmMember::new(SwarmId::PLAYER),
                    Transform::from_xyz(
                        252. + x as f32 * 72.,
                        252. + y as f32 * 72.,
                        GAMEPLAY_SPRITE_Z + 1.,
                    ),
                ))
                .id();
            if x == 0 && y == -1 {
                blocker = Some(bot);
            }
        }
    }
    let caption = world
        .spawn((
            Text2d::new("OUTPUT FINISHED / EXITS OCCUPIED"),
            TextFont {
                font_size: 20.,
                ..default()
            },
            Transform::from_xyz(252., 395., GAMEPLAY_SPRITE_Z),
        ))
        .id();
    world.insert_resource(ExitScene {
        facility,
        blocker: blocker.unwrap(),
        caption,
        phase: 0,
        released_frame: 0,
    });
}
