use super::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{PlannedKind, PlannedStructure, StructureClearing, planned_visual_color},
    navigation::Obstacle,
    structure_sprites::{StructureSprites, StructureVisual, StructureVisualState},
};

const CENTER: Vec2 = Vec2::new(252., 252.);
const EXIT: Vec2 = Vec2::new(324., 252.);

#[derive(Resource)]
struct ClearingScene {
    plan: Entity,
    footprint: Obstacle,
    walls: [Obstacle; 3],
    bots: [Entity; 2],
    stationary: Entity,
    previous: [Vec2; 2],
    phase: Phase,
    ticks: u32,
}

pub(super) fn configure(app: &mut App) {
    app.add_systems(First, (prepare.run_if(run_once), release_phase).chain())
        .add_systems(FixedPostUpdate, verify_tick);
}

fn prepare(world: &mut World) {
    let swarm = super::prepare_world(world, CENTER);
    let plan_transform = Transform::from_translation(CENTER.extend(GAMEPLAY_SPRITE_Z))
        .with_scale(Vec3::new(1.125, 1.125, 1.));
    let walls = [
        (Vec2::new(180., 252.), 1.125),
        (Vec2::new(288., 180.), 4.5),
        (Vec2::new(288., 324.), 4.5),
    ]
    .map(|(position, width)| {
        let transform = plan_transform
            .with_translation(position.extend(GAMEPLAY_SPRITE_Z))
            .with_scale(Vec3::new(width, 1.125, 1.));
        world.spawn((
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.)),
            transform,
        ));
        Obstacle::structure(&transform)
    });
    let kind = PlannedKind::SourceStockpile;
    let image = world
        .resource::<StructureSprites>()
        .handle(kind, StructureVisualState::Planned);
    let plan = world
        .spawn((
            PlannedStructure::new(kind, IVec2::ZERO).with_work_remaining(0),
            OwnerSwarm(swarm),
            StructureVisual::planned(kind),
            Sprite {
                image,
                color: planned_visual_color(),
                custom_size: Some(Vec2::splat(64.)),
                ..default()
            },
            plan_transform,
        ))
        .id();
    let previous = [CENTER, CENTER + Vec2::X * 25.];
    let bots = previous.map(|position| super::spawn_hauler(world, position));
    let stationary = super::spawn_hauler(world, EXIT);
    world.insert_resource(ClearingScene {
        plan,
        footprint: Obstacle::structure(&plan_transform),
        walls,
        bots,
        stationary,
        previous,
        phase: Phase::Before,
        ticks: 0,
    });
    world.resource_mut::<Time<Virtual>>().pause();
    info!("CLEARING before: two occupants and one occupied exit; paint corridor -3 -3 to start");
}

fn release_phase(world: &mut World) {
    let scene = world.resource::<ClearingScene>();
    let plan = scene.plan;
    let phase = scene.phase;
    let Some(next) = release_after_corridor(world, phase, None) else {
        return;
    };
    if next == Phase::Approaching {
        world
            .entity_mut(plan)
            .insert(StructureClearing::awaiting_validation(EXIT));
    }
    world.resource_mut::<ClearingScene>().phase = next;
    world.resource_mut::<Time<Virtual>>().unpause();
}

fn verify_tick(
    mut scene: ResMut<ClearingScene>,
    positions: Query<&Transform>,
    plans: Query<(), With<StructureClearing>>,
    stockpiles: Query<(), With<Stockpile>>,
    mut time: ResMut<Time<Virtual>>,
) {
    if !matches!(scene.phase, Phase::Approaching | Phase::Resuming) {
        return;
    }
    scene.ticks += 1;
    let current = scene
        .bots
        .map(|entity| positions.get(entity).unwrap().translation.truncate());
    assert_eq!(
        positions
            .get(scene.stationary)
            .unwrap()
            .translation
            .truncate(),
        EXIT
    );
    for (&start, &end) in scene.previous.iter().zip(&current) {
        assert!(
            start.distance(end) <= 5.001,
            "clearing displaced an occupant: {start:?} -> {end:?}"
        );
        assert!(
            scene
                .walls
                .iter()
                .all(|wall| wall.segment_clear(start, end)),
            "swept wall collision: {start:?} -> {end:?}"
        );
    }
    scene.previous = current;
    let outside = current
        .iter()
        .filter(|&&point| scene.footprint.admits_body(point))
        .count();
    let active = stockpiles.contains(scene.plan);
    assert!(
        !active || outside == scene.bots.len(),
        "activated while occupied: {current:?}"
    );
    assert!(scene.ticks <= 180, "clearing stalled: {current:?}");
    if scene.phase == Phase::Approaching && outside == 1 {
        assert!(plans.contains(scene.plan));
        assert!(current.iter().any(|point| point.distance(EXIT) < 20.));
        scene.phase = Phase::Passing;
        time.pause();
        info!(
            "CLEARING passing: tick {}, positions {current:?}; paint corridor -2 -3 to resume",
            scene.ticks
        );
    } else if scene.phase == Phase::Resuming && active {
        scene.phase = Phase::Arrived;
        time.pause();
        info!(
            "CLEARING activated: tick {}, positions {current:?}",
            scene.ticks
        );
    }
}
