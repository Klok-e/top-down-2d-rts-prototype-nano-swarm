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
    captured_effect: Option<(f32, Vec3)>,
    pending_capture: Option<&'static str>,
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
        captured_effect: None,
        pending_capture: None,
    });
}

pub fn construction_clearing(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame < 2 {
        return TestFlow::Continue;
    }
    if ctx.frame == 2 {
        prepare(ctx.world, false);
        ctx.world.resource_mut::<Time<Virtual>>().pause();
        return TestFlow::Screenshot("construction_clearing_occupied".into());
    }
    let scene = ctx.world.resource::<Scene>();
    let (plan, bot, phase) = (scene.plan, scene.bot, scene.phase);
    if phase == 0 {
        assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
        assert!(ctx.world.get::<PlannedStructure>(plan).is_some());
        assert!(ctx.world.get::<Stockpile>(plan).is_none());
        assert_eq!(
            ctx.world
                .get::<Transform>(bot)
                .unwrap()
                .translation
                .truncate(),
            Vec2::new(252., 252.),
        );
        ctx.world.resource_mut::<Time<Virtual>>().unpause();
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
        ctx.world.resource_mut::<Time<Virtual>>().pause();
        return TestFlow::Screenshot("construction_clearing_activated".into());
    } else if phase == 2 {
        assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
        assert!(ctx.world.get::<Stockpile>(plan).is_some());
        assert!(ctx.world.get::<PlannedStructure>(plan).is_none());
        assert!(ctx.world.get::<StructureClearing>(plan).is_none());
        let obstacle = top_down_2d_rts_prototype_nano_swarm::navigation::Obstacle::structure(
            ctx.world.get::<Transform>(plan).unwrap(),
        );
        assert!(
            obstacle.admits_body(
                ctx.world
                    .get::<Transform>(bot)
                    .unwrap()
                    .translation
                    .truncate()
            )
        );
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
    let (plan, phase, captured_effect) = (scene.plan, scene.phase, scene.captured_effect);
    // Pausing in First can leave one prepared fixed tick; capture only after it settles.
    if let Some(name) = scene.pending_capture {
        assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
        let (sprite, transform) = ctx
            .world
            .query_filtered::<(&Sprite, &Transform), With<CancelledPlanVisual>>()
            .single(ctx.world)
            .expect("cancellation phase must remain visible after pausing");
        let snapshot = (sprite.color.to_srgba().alpha, transform.scale);
        let mut scene = ctx.world.resource_mut::<Scene>();
        scene.pending_capture = None;
        scene.captured_effect = Some(snapshot);
        return TestFlow::Screenshot(name.into());
    }
    if let Some((alpha, scale)) = captured_effect {
        assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
        let (sprite, transform) = ctx
            .world
            .query_filtered::<(&Sprite, &Transform), With<CancelledPlanVisual>>()
            .single(ctx.world)
            .expect("captured cancellation effect must survive readback");
        assert!((sprite.color.to_srgba().alpha - alpha).abs() < 1e-5);
        assert!(transform.scale.abs_diff_eq(scale, 1e-5));
        assert!(ctx.world.get_entity(plan).is_err());
        ctx.world.resource_mut::<Scene>().captured_effect = None;
        ctx.world.resource_mut::<Time<Virtual>>().unpause();
    }
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
        let (sprite, transform) = ctx
            .world
            .query_filtered::<(&Sprite, &Transform), With<CancelledPlanVisual>>()
            .single(ctx.world)
            .unwrap();
        let color = sprite.color.to_srgba();
        assert!(color.red > 0.95 && color.green < 0.1 && color.blue < 0.1);
        let alpha = color.alpha;
        let scale = transform.scale;
        match phase {
            1 => assert!(
                alpha > 0.99 && scale.x > 2.24,
                "pulse is full size and opaque"
            ),
            2 => assert!((0.70..=0.84).contains(&alpha) && scale.x < 2.0),
            3 => assert!((0.25..=0.42).contains(&alpha) && scale.x < 1.0),
            _ => unreachable!(),
        }
        ctx.world.resource_mut::<Scene>().pending_capture = Some(name);
        ctx.world.resource_mut::<Time<Virtual>>().pause();
        ctx.world.resource_mut::<Scene>().phase += 1;
    }
    assert!(ctx.frame < 150, "cancellation visual phases not observed");
    TestFlow::Continue
}
