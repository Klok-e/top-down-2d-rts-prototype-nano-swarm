//! Full-app clearing and cancellation presentation, driven by simulation ticks.
use crate::harness::{TestContext, TestFlow, clear_nanobots_and_sprite_entities};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::structure_overlay::CancelledPlanVisual;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::CameraZoom2d,
    intent::IntentGrid,
    nanobot::{
        Commitment, Health, Nanobot, NanobotType, OpponentIntentController, PlannedKind,
        PlannedStructure, Structure, StructureClearing, StructureKind, SwarmId, SwarmMember,
        VelocityComponent,
    },
    resources::Stockpile,
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
};
#[derive(Resource)]
struct Scene {
    plan: Entity,
    bot: Entity,
    phase: u8,
}

fn prepare(world: &mut World, cancel: bool) {
    clear_nanobots_and_sprite_entities(world);
    let entities = world
        .query_filtered::<Entity, Or<(With<Node>, With<Mesh2d>)>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        let _ = world.despawn(entity);
    }
    let controllers = world
        .query_filtered::<Entity, With<OpponentIntentController>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in controllers {
        world
            .entity_mut(entity)
            .remove::<OpponentIntentController>();
    }
    world.insert_resource(IntentGrid::new(8, 8));
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = 252.;
        transform.translation.y = 252.;
        zoom.zoom = 0.5;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 0.5;
        }
    }
    let kind = PlannedKind::SinkStockpile;
    let mut sprite = world
        .resource::<StructureSprites>()
        .sprite(kind, StructureVisualState::Planned);
    sprite.custom_size = Some(Vec2::splat(64.));
    let plan = world
        .spawn((
            PlannedStructure::new(kind, IVec2::ZERO),
            sprite,
            StructureVisual::planned(kind),
            Transform::from_xyz(252., 252., 1.).with_scale(Vec3::new(2.25, 2.25, 1.)),
        ))
        .id();
    {
        let mut state = world.get_mut::<PlannedStructure>(plan).unwrap();
        *state = state.with_work_remaining(0);
    }
    let bot = world
        .spawn((
            Nanobot {},
            NanobotType::Defender,
            Commitment::Working,
            Health::default(),
            VelocityComponent::default(),
            SwarmMember::new(SwarmId::PLAYER),
            Transform::from_xyz(252., 252., 2.),
        ))
        .id();
    if cancel {
        world.spawn((
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.5, 0.3, 0.2), Vec2::splat(64.)),
            Transform::from_xyz(108., 252., 1.),
        ));
    }
    world.insert_resource(Scene {
        plan,
        bot,
        phase: 0,
    });
}

pub fn construction_clearing(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare(ctx.world, false);
        return TestFlow::Screenshot("construction_clearing_occupied".into());
    }
    let scene = ctx.world.resource::<Scene>();
    let (plan, bot, phase) = (scene.plan, scene.bot, scene.phase);
    if phase == 0 {
        ctx.world
            .entity_mut(plan)
            .insert(StructureClearing::awaiting_validation(Vec2::new(
                108., 252.,
            )));
        ctx.world.resource_mut::<Scene>().phase = 1;
    } else if phase == 1 && ctx.world.get::<Stockpile>(plan).is_some() {
        let transform = ctx.world.get::<Transform>(plan).unwrap();
        let position = ctx
            .world
            .get::<Transform>(bot)
            .unwrap()
            .translation
            .truncate();
        assert!(
            top_down_2d_rts_prototype_nano_swarm::navigation::Obstacle::structure(transform)
                .admits_body(position)
        );
        ctx.world.resource_mut::<Scene>().phase = 2;
        return TestFlow::Screenshot("construction_clearing_activated".into());
    } else if phase == 2 {
        return TestFlow::Exit;
    }
    assert!(ctx.frame < 500, "occupied plan failed to clear");
    TestFlow::Continue
}

pub fn construction_cancellation(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare(ctx.world, true);
    }
    let scene = ctx.world.resource::<Scene>();
    let (plan, phase) = (scene.plan, scene.phase);
    if phase == 0 {
        ctx.world
            .entity_mut(plan)
            .insert(StructureClearing::awaiting_validation(Vec2::new(
                108., 252.,
            )));
        ctx.world.resource_mut::<Scene>().phase = 1;
    }
    let opacity = ctx
        .world
        .query_filtered::<&Sprite, With<CancelledPlanVisual>>()
        .iter(ctx.world)
        .next()
        .map(|sprite| sprite.color.to_srgba().alpha);
    let capture = match (phase, opacity) {
        (1, Some(_)) => Some("construction_cancellation_pulse"),
        (2, Some(alpha)) if alpha <= 0.84 => Some("construction_cancellation_collapse"),
        (3, Some(alpha)) if alpha <= 0.42 => Some("construction_cancellation_fade"),
        (4, None) => return TestFlow::Exit,
        _ => None,
    };
    if let Some(name) = capture {
        assert!(ctx.world.get_entity(plan).is_err());
        ctx.world.resource_mut::<Scene>().phase += 1;
        return TestFlow::Screenshot(name.into());
    }
    assert!(ctx.frame < 150, "cancellation visual phases not observed");
    TestFlow::Continue
}

pub fn production_exit(ctx: &mut TestContext) -> TestFlow {
    use top_down_2d_rts_prototype_nano_swarm::{
        game_settings::GameSettings,
        nanobot::{DirectMovementComponent, PRODUCTION_TICKS_PER_BOT, ProductionFacility},
    };
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare(ctx.world, false);
        let scene = ctx.world.resource::<Scene>();
        let (plan, bot) = (scene.plan, scene.bot);
        ctx.world.despawn(plan);
        ctx.world.despawn(bot);
        let mut facility = ProductionFacility::new();
        facility.current_target = Some(NanobotType::Hauler);
        facility.progress = PRODUCTION_TICKS_PER_BOT;
        let facility = ctx
            .world
            .spawn((
                facility,
                Sprite::from_color(Color::srgb(0.3, 0.5, 0.8), Vec2::splat(64.)),
                Transform::from_xyz(252., 252., 1.).with_scale(Vec3::new(1.125, 1.125, 1.)),
            ))
            .id();
        let mut opening = None;
        for y in -1..=1 {
            for x in -1..=1 {
                if x != 0 || y != 0 {
                    let bot = ctx
                        .world
                        .spawn((
                            Nanobot {},
                            NanobotType::Worker,
                            Commitment::Working,
                            Health::default(),
                            VelocityComponent::default(),
                            SwarmMember::new(SwarmId::PLAYER),
                            Transform::from_xyz(252. + x as f32 * 72., 252. + y as f32 * 72., 2.),
                        ))
                        .id();
                    if x == 0 && y == -1 {
                        opening = Some(bot);
                    }
                }
            }
        }
        ctx.world.resource_mut::<GameSettings>().bot_speed = 0.;
        ctx.world.insert_resource(Scene {
            plan: facility,
            bot: opening.unwrap(),
            phase: 0,
        });
        return TestFlow::Continue;
    }
    let scene = ctx.world.resource::<Scene>();
    let (facility, bot, phase) = (scene.plan, scene.bot, scene.phase);
    if phase == 0 {
        assert_eq!(
            ctx.world
                .query_filtered::<Entity, With<Nanobot>>()
                .iter(ctx.world)
                .count(),
            8
        );
        assert!(
            ctx.world
                .get::<ProductionFacility>(facility)
                .unwrap()
                .current_target
                .is_some()
        );
        ctx.world.resource_mut::<Scene>().phase = 1;
        return TestFlow::Screenshot("production_exit_waiting".into());
    }
    if phase == 1 {
        ctx.world.resource_mut::<GameSettings>().bot_speed = 5.;
        ctx.world.entity_mut(bot).insert(DirectMovementComponent {
            xy: Vec2::new(252., 36.),
            stop_radius: 0.,
            interaction: None,
            speed: None,
        });
        ctx.world.resource_mut::<Scene>().phase = 2;
    } else if phase == 2
        && ctx
            .world
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target
            .is_none()
    {
        assert_eq!(
            ctx.world
                .query_filtered::<Entity, With<Nanobot>>()
                .iter(ctx.world)
                .count(),
            9
        );
        let center = Vec2::new(252., 252.);
        for (kind, transform) in ctx
            .world
            .query::<(&NanobotType, &Transform)>()
            .iter(ctx.world)
        {
            if *kind == NanobotType::Hauler {
                assert!(transform.translation.truncate().distance(center) > 68.);
            }
        }
        ctx.world.resource_mut::<GameSettings>().bot_speed = 0.;
        ctx.world.resource_mut::<Scene>().phase = 3;
        return TestFlow::Screenshot("production_exit_released".into());
    } else if phase == 3 {
        return TestFlow::Exit;
    }
    assert!(
        ctx.frame < 300,
        "finished output failed to leave the facility"
    );
    TestFlow::Continue
}
