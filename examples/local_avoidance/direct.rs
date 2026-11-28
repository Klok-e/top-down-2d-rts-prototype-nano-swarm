use super::*;
use top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent;

const START: Vec2 = Vec2::new(180., 108.);
const STARTS: [Vec2; 2] = [Vec2::new(660., 108.), Vec2::new(680., 108.)];
const WESTBOUND_GOAL: Vec2 = Vec2::new(230., 108.);

#[derive(Resource)]
struct DirectScene {
    delivery: Entity,
    source: Entity,
    facility: Entity,
    counterflow: [Entity; 2],
    phase: Phase,
    ticks: u32,
    greatest_offset: f32,
}

pub(super) fn configure(app: &mut App) {
    app.add_systems(First, (prepare.run_if(run_once), release_phase).chain())
        .add_systems(FixedPostUpdate, verify_tick);
}

fn prepare(world: &mut World) {
    let swarm = prepare_world(world, Vec2::new(535., 110.));
    let mut grid = world.resource_mut::<IntentGrid>();
    for cell in [IVec2::ZERO, IVec2::X] {
        grid.erase(cell, IntentKind::Gather, SwarmId::PLAYER);
    }
    for (y, color) in [
        (108., Color::srgb(0.18, 0.22, 0.26)),
        (180., Color::srgb(0.13, 0.22, 0.23)),
        (36., Color::srgb(0.13, 0.22, 0.23)),
    ] {
        world.spawn((
            Sprite::from_color(color, Vec2::new(638., 32.)),
            Transform::from_xyz(504., y, GAMEPLAY_SPRITE_Z - 1.),
        ));
    }
    let source = world
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 0,
                capacity: 100,
                radius: 36.,
            },
            StockpileRole::Source,
            OwnerSwarm(swarm),
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.)),
            Transform::from_xyz(1044., 108., GAMEPLAY_SPRITE_Z)
                .with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let facility = world
        .spawn((
            ProductionFacility::new(),
            Structure::new(StructureKind::Basic),
            OwnerSwarm(swarm),
            Sprite::from_color(Color::srgb(0.2, 0.42, 0.6), Vec2::splat(64.)),
            Transform::from_xyz(900., 108., GAMEPLAY_SPRITE_Z)
                .with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let delivery = spawn_hauler(world, START);
    let counterflow = STARTS.map(|point| spawn_hauler(world, point));
    world.resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        SHIPMENT,
    );
    world.insert_resource(DirectScene {
        delivery,
        source,
        facility,
        counterflow,
        phase: Phase::Before,
        ticks: 0,
        greatest_offset: 0.,
    });
    world.resource_mut::<Time<Virtual>>().pause();
    info!(
        "DIRECT before: eastbound delivery and two westbound routes; paint corridor -3 -3 to start"
    );
}

fn release_phase(world: &mut World) {
    let scene = world.resource::<DirectScene>();
    let (cell, next) = match scene.phase {
        Phase::Before => (IVec2::new(-3, -3), Phase::Approaching),
        Phase::Passing => (IVec2::new(-2, -3), Phase::Resuming),
        _ => return,
    };
    if !world
        .resource::<IntentGrid>()
        .cell(cell)
        .is_some_and(|cell| cell.has_owned(IntentKind::Corridor, SwarmId::PLAYER))
    {
        return;
    }
    world
        .resource_mut::<IntentGrid>()
        .erase(cell, IntentKind::Corridor, SwarmId::PLAYER);
    if next == Phase::Approaching {
        let counterflow = world.resource::<DirectScene>().counterflow;
        for bot in counterflow {
            world.entity_mut(bot).insert(DirectMovementComponent {
                xy: WESTBOUND_GOAL,
                stop_radius: 0.,
                speed: None,
                interaction: None,
            });
        }
    }
    world.resource_mut::<DirectScene>().phase = next;
    world.resource_mut::<Time<Virtual>>().unpause();
}

fn verify_tick(world: &mut World) {
    world.resource_scope(|world, mut scene: Mut<DirectScene>| {
        if !matches!(scene.phase, Phase::Approaching | Phase::Resuming) {
            return;
        }
        scene.ticks += 1;
        if scene.ticks == 2 {
            let region = InteractionRegion::structure(world.get::<Transform>(scene.facility).unwrap());
            let mut reservation = LogisticsReservation::new(
                scene.source,
                scene.facility,
                ResourceKind::Minerals,
                SHIPMENT,
            );
            reservation.source_remaining = 0;
            world.entity_mut(scene.delivery).insert((
                HaulerAssignment {
                    source: scene.source,
                    sink: scene.facility,
                    pickup_role: StockpileRole::Source,
                },
                Cargo { kind: ResourceKind::Minerals, amount: SHIPMENT },
                reservation,
                region.movement_from(START),
            ));
        }

        let position = |bot| world.get::<Transform>(bot).unwrap().translation.truncate();
        let current = position(scene.delivery);
        scene.greatest_offset = scene.greatest_offset.max((current.y - START.y).abs());
        if scene.phase == Phase::Approaching && current.x >= 480. {
            let westbound = scene.counterflow.map(position);
            assert!(westbound.iter().zip(STARTS).all(|(point, start)| point.x < start.x - 100. && (point.y - start.y).abs() < 1.));
            assert!((current.y - START.y).abs() > 60., "delivery did not take the quieter bend: {current}");
            info!("DIRECT passing: tick {}, delivery {current:?}, westbound {westbound:?}; paint corridor -2 -3 to resume", scene.ticks);
            scene.phase = Phase::Passing;
            world.resource_mut::<Time<Virtual>>().pause();
        } else if scene.phase == Phase::Resuming
            && world.get::<ProductionFacility>(scene.facility).unwrap().input_amount == SHIPMENT
        {
            assert!((current.y - START.y).abs() < 2., "delivery did not return to the work position: {current}");
            info!("DIRECT arrived: tick {}, delivered {SHIPMENT} minerals at {current:?}; maximum lane offset {:.2}", scene.ticks, scene.greatest_offset);
            scene.phase = Phase::Arrived;
            world.resource_mut::<Time<Virtual>>().pause();
        }
        assert!(
            scene.phase == Phase::Arrived || scene.ticks < 240,
            "delivery did not finish: position {current}, cargo {:?}, movement {:?}",
            world.get::<Cargo>(scene.delivery),
            world.get::<DirectMovementComponent>(scene.delivery),
        );
    });
}
