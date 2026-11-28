use super::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::DirectMovementComponent, navigation::Navigation,
};

#[derive(Clone, Copy, Resource)]
pub(super) struct LaneLayout {
    label: &'static str,
    start: Vec2,
    westbound_starts: &'static [Vec2],
    westbound_goal: Vec2,
    source: Vec2,
    facility: Vec2,
    camera: Vec2,
    passing_x: f32,
    walls: &'static [Vec2],
    quiet_lane_y: Option<f32>,
}

pub(super) const DIRECT: LaneLayout = LaneLayout {
    label: "DIRECT",
    start: Vec2::new(180., 108.),
    westbound_starts: &[Vec2::new(660., 108.), Vec2::new(680., 108.)],
    westbound_goal: Vec2::new(230., 108.),
    source: Vec2::new(1044., 108.),
    facility: Vec2::new(900., 108.),
    camera: Vec2::new(535., 110.),
    passing_x: 480.,
    walls: &[],
    quiet_lane_y: None,
};

pub(super) const SEARCHED: LaneLayout = LaneLayout {
    label: "SEARCHED",
    start: Vec2::new(108., 251.),
    westbound_starts: &[
        Vec2::new(376., 180.),
        Vec2::new(401., 180.),
        Vec2::new(427., 180.),
    ],
    westbound_goal: Vec2::new(108., 180.),
    source: Vec2::new(684., 252.),
    facility: Vec2::new(540., 252.),
    camera: Vec2::new(380., 252.),
    passing_x: 288.,
    walls: &[
        Vec2::new(288., 108.),
        Vec2::new(288., 252.),
        Vec2::new(288., 396.),
    ],
    quiet_lane_y: Some(324.),
};

#[derive(Resource)]
struct LaneScene {
    delivery: Entity,
    source: Entity,
    facility: Entity,
    counterflow: Vec<Entity>,
    phase: Phase,
    ticks: u32,
    previous: Vec2,
    greatest_offset: f32,
}

pub(super) fn configure(app: &mut App, layout: LaneLayout) {
    app.insert_resource(layout)
        .add_systems(First, (prepare.run_if(run_once), release_phase).chain())
        .add_systems(FixedPostUpdate, verify_tick);
}

fn prepare(world: &mut World) {
    let layout = *world.resource::<LaneLayout>();
    let swarm = prepare_world(world, layout.camera);
    let mut grid = world.resource_mut::<IntentGrid>();
    for cell in [IVec2::ZERO, IVec2::X] {
        grid.erase(cell, IntentKind::Gather, SwarmId::PLAYER);
    }
    for &center in layout.walls {
        world.spawn((
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.)),
            Transform::from_translation(center.extend(GAMEPLAY_SPRITE_Z))
                .with_scale(Vec3::new(2.25, 1.125, 1.)),
        ));
    }
    if layout.walls.is_empty() {
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
            Transform::from_translation(layout.source.extend(GAMEPLAY_SPRITE_Z))
                .with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let facility = world
        .spawn((
            ProductionFacility::new(),
            Structure::new(StructureKind::Basic),
            OwnerSwarm(swarm),
            Sprite::from_color(Color::srgb(0.2, 0.42, 0.6), Vec2::splat(64.)),
            Transform::from_translation(layout.facility.extend(GAMEPLAY_SPRITE_Z))
                .with_scale(Vec3::new(1.125, 1.125, 1.)),
        ))
        .id();
    let delivery = spawn_hauler(world, layout.start);
    let counterflow = layout
        .westbound_starts
        .iter()
        .map(|&point| spawn_hauler(world, point))
        .collect();
    world.resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        SHIPMENT,
    );
    world.insert_resource(LaneScene {
        delivery,
        source,
        facility,
        counterflow,
        phase: Phase::Before,
        ticks: 0,
        previous: layout.start,
        greatest_offset: 0.,
    });
    world.resource_mut::<Time<Virtual>>().pause();
    info!(
        "{} before: eastbound delivery and westbound routes; paint corridor -3 -3 to start",
        layout.label
    );
}

fn release_phase(world: &mut World) {
    let scene = world.resource::<LaneScene>();
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
        let goal = world.resource::<LaneLayout>().westbound_goal;
        let counterflow = world.resource::<LaneScene>().counterflow.clone();
        for bot in counterflow {
            world.entity_mut(bot).insert(DirectMovementComponent {
                xy: goal,
                stop_radius: 0.,
                speed: None,
                interaction: None,
            });
        }
    }
    world.resource_mut::<LaneScene>().phase = next;
    world.resource_mut::<Time<Virtual>>().unpause();
}

fn verify_tick(world: &mut World) {
    let layout = *world.resource::<LaneLayout>();
    world.resource_scope(|world, mut scene: Mut<LaneScene>| {
        if !matches!(scene.phase, Phase::Approaching | Phase::Passing | Phase::Resuming) {
            return;
        }
        scene.ticks += 1;
        if scene.ticks == 2 {
            let region = InteractionRegion::structure(world.get::<Transform>(scene.facility).unwrap());
            let navigation = world.resource::<Navigation>();
            assert_eq!(
                navigation.segment_clear(layout.start, region.approach(layout.start)),
                layout.quiet_lane_y.is_none(),
                "the two fixtures must exercise different route choices",
            );
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
                region.movement_from(layout.start),
            ));
        }

        let position = |bot| world.get::<Transform>(bot).unwrap().translation.truncate();
        let current = position(scene.delivery);
        let navigation = world.resource::<Navigation>();
        assert!(navigation.segment_clear(scene.previous, current));
        scene.previous = current;
        let offset = (current.y - layout.start.y).abs();
        scene.greatest_offset = scene.greatest_offset.max(offset);
        if scene.phase == Phase::Approaching && current.x >= layout.passing_x {
            let westbound: Vec<_> = scene.counterflow.iter().copied().map(position).collect();
            assert!(westbound.iter().zip(layout.westbound_starts).all(|(point, start)|
                point.x < start.x - 100. && (point.y - start.y).abs() < 1.));
            if let Some(y) = layout.quiet_lane_y {
                assert!((current.y - y).abs() < 2., "delivery used the busier opening: {current}");
            } else {
                assert!(offset > 60., "delivery did not take the quieter bend: {current}");
            }
            info!("{} passing: tick {}, delivery {current:?}, westbound {westbound:?}; paint corridor -2 -3 to resume", layout.label, scene.ticks);
            scene.phase = Phase::Passing;
            world.resource_mut::<Time<Virtual>>().pause();
        } else if scene.phase == Phase::Resuming
            && world.get::<ProductionFacility>(scene.facility).unwrap().input_amount == SHIPMENT
        {
            assert!((current.y - layout.facility.y).abs() < 2., "delivery did not return to the work position: {current}");
            info!("{} arrived: tick {}, delivered {SHIPMENT} minerals at {current:?}; maximum lane offset {:.2}", layout.label, scene.ticks, scene.greatest_offset);
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
