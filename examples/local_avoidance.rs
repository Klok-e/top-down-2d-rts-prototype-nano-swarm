//! Full-game headless replays of deliveries and Structure Clearing.
//!
//! The replay driver uses two Corridor paints to pause on friendly passage and
//! the completed outcome. See docs/verification/navigation.md for the commands.
#[path = "local_avoidance/clearing.rs"]
mod clearing;
#[path = "local_avoidance/direct.rs"]
mod direct;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    agent_control::AgentControlConfig,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, Commitment, HaulerAssignment, HaulerLoading, Health, InteractionRegion,
        LogisticsReservation, MaterialWorkProgress, Nanobot, NanobotType, OwnerSwarm,
        ProductionFacility, Structure, StructureKind, Swarm, SwarmId, SwarmMember,
        VelocityComponent, WorkBlocked,
    },
    reachability::SwarmReachabilityMap,
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
    scenario_selection::Scenario,
    session_lifecycle::SessionEntity,
    tuning::Tuning,
};

use top_down_2d_rts_prototype_nano_swarm::runtime::{RuntimeOptions, build_runtime_app};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Before,
    Approaching,
    Passing,
    Resuming,
    Arrived,
}

struct Delivery {
    bot: Entity,
    facility: Entity,
    direction: f32,
}

#[derive(Resource)]
struct TrafficScene {
    deliveries: [Delivery; 2],
    source: Entity,
    loading: Entity,
    previous: [Vec2; 2],
    phase: Phase,
    ticks: u32,
}

const LOADING_POSITION: Vec2 = Vec2::new(432., 324.);
const SHIPMENT: u32 = 2;

fn main() {
    let control = AgentControlConfig::from_environment().expect("agent control socket");
    let mut app = build_runtime_app(
        RuntimeOptions {
            headless: true,
            scenario: Some(Scenario::Standard),
            tuning_file: std::env::var_os("NANO_SWARM_REPLAY_TUNING").map_or_else(
                top_down_2d_rts_prototype_nano_swarm::tuning::default_path,
                Into::into,
            ),
            width: 1280,
            height: 720,
            ..Default::default()
        },
        Some(control),
    )
    .expect("headless runtime");
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1. / 60.),
    ));
    match std::env::var("NANO_SWARM_REPLAY_SCENE").as_deref() {
        Ok("clearing") => clearing::configure(&mut app),
        Ok("direct") => direct::configure(&mut app),
        _ => {
            app.add_systems(First, (prepare.run_if(run_once), release_phase).chain())
                .add_systems(FixedPostUpdate, verify_tick);
        }
    }
    app.run();
}

fn release_phase(world: &mut World) {
    let phase = world.resource::<TrafficScene>().phase;
    let (cell, next) = match phase {
        Phase::Before => (IVec2::new(-3, -3), Phase::Approaching),
        Phase::Passing => (IVec2::new(-2, -3), Phase::Resuming),
        _ => return,
    };
    if !world
        .resource::<IntentGrid>()
        .cell(cell)
        .is_some_and(|cell| cell.has(IntentKind::Corridor))
    {
        return;
    }
    if phase == Phase::Before {
        world.resource_scope(|world, scene: Mut<TrafficScene>| {
            for delivery in &scene.deliveries {
                let position = world
                    .get::<Transform>(delivery.bot)
                    .unwrap()
                    .translation
                    .truncate();
                let region = InteractionRegion::structure(
                    world.get::<Transform>(delivery.facility).unwrap(),
                );
                let mut reservation = LogisticsReservation::new(
                    scene.source,
                    delivery.facility,
                    ResourceKind::Minerals,
                    SHIPMENT,
                );
                reservation.source_remaining = 0;
                world.entity_mut(delivery.bot).insert((
                    HaulerAssignment {
                        source: scene.source,
                        sink: delivery.facility,
                        pickup_role: StockpileRole::Source,
                    },
                    Cargo {
                        kind: ResourceKind::Minerals,
                        amount: SHIPMENT,
                    },
                    reservation,
                    region.movement_from(position),
                ));
            }
            let loading_capacity = world.resource::<Tuning>().logistics.hauler_carry_capacity;
            world.entity_mut(scene.loading).insert((
                HaulerAssignment {
                    source: scene.source,
                    sink: scene.deliveries[0].facility,
                    pickup_role: StockpileRole::Source,
                },
                Cargo::empty(ResourceKind::Minerals),
                MaterialWorkProgress::default(),
                HaulerLoading,
                LogisticsReservation::new(
                    scene.source,
                    scene.deliveries[0].facility,
                    ResourceKind::Minerals,
                    loading_capacity,
                ),
            ));
        });
    }
    world.resource_mut::<TrafficScene>().phase = next;
    world.resource_mut::<Time<Virtual>>().unpause();
    info!("TRAFFIC running phase {next:?}");
}

fn verify_tick(
    mut scene: ResMut<TrafficScene>,
    positions: Query<&Transform>,
    loading: Query<(&Cargo, &MaterialWorkProgress, Has<WorkBlocked>), With<HaulerLoading>>,
    facilities: Query<&ProductionFacility>,
    tuning: Res<Tuning>,
    mut time: ResMut<Time<Virtual>>,
) {
    if !matches!(scene.phase, Phase::Approaching | Phase::Resuming) {
        return;
    }
    scene.ticks += 1;
    let current = scene
        .deliveries
        .each_ref()
        .map(|delivery| positions.get(delivery.bot).unwrap().translation.truncate());
    let received = scene
        .deliveries
        .each_ref()
        .map(|delivery| facilities.get(delivery.facility).unwrap().input_amount);
    for (index, &end) in current.iter().enumerate() {
        let start = scene.previous[index];
        assert!(
            (end.x - start.x) * scene.deliveries[index].direction >= -0.001,
            "ordinary traffic retreated at tick {}: {start:?} -> {end:?}",
            scene.ticks
        );
        for center in [Vec2::new(432., 252.), Vec2::new(432., 396.)] {
            assert!(
                segment_rectangle_distance(start, end, center) >= 33.99,
                "swept structure collision at tick {}: {start:?} -> {end:?}",
                scene.ticks
            );
        }
    }
    scene.previous = current;
    let (cargo, material, blocked) = loading
        .get(scene.loading)
        .expect("working Hauler is still loading");
    assert_eq!(
        positions.get(scene.loading).unwrap().translation.truncate(),
        LOADING_POSITION,
        "passing traffic moved the loading Hauler"
    );
    assert!(!blocked, "passing traffic blocked loading");
    assert_eq!(
        cargo.amount * tuning.logistics.material_work_ticks + material.ticks_elapsed,
        scene.ticks,
        "loading must progress on every tick of the delivery"
    );

    assert!(
        scene.ticks < 200,
        "deliveries stalled: {current:?}, received {received:?}"
    );
    if scene.phase == Phase::Approaching
        && current[0].x - current[1].x >= 44.
        && current
            .iter()
            .all(|position| position.distance(LOADING_POSITION) < 34.)
    {
        scene.phase = Phase::Passing;
        time.pause();
        info!(
            "TRAFFIC passing: tick {}, positions {current:?}, loaded minerals {}, work ticks {}; paint corridor -2 -3 to resume",
            scene.ticks, cargo.amount, material.ticks_elapsed
        );
    } else if scene.phase == Phase::Resuming && received == [SHIPMENT; 2] {
        scene.phase = Phase::Arrived;
        time.pause();
        info!(
            "TRAFFIC arrived: tick {}, positions {current:?}, delivered {received:?}; loading and swept clearance checks passed every fixed tick",
            scene.ticks
        );
    }
}

// Independent distance to the authored 288 x 72 rectangle, including segment crossings.
fn segment_rectangle_distance(a: Vec2, b: Vec2, center: Vec2) -> f32 {
    let low = center - Vec2::new(144., 36.);
    let high = center + Vec2::new(144., 36.);
    let delta = b - a;
    let mut enter: f32 = 0.;
    let mut leave: f32 = 1.;
    for axis in 0..2 {
        if delta[axis].abs() < 1e-8 {
            if a[axis] < low[axis] || a[axis] > high[axis] {
                enter = 2.;
            }
        } else {
            let t0 = (low[axis] - a[axis]) / delta[axis];
            let t1 = (high[axis] - a[axis]) / delta[axis];
            enter = enter.max(t0.min(t1));
            leave = leave.min(t0.max(t1));
        }
    }
    if enter <= leave {
        return 0.;
    }
    let mut distance = a
        .distance(a.clamp(low, high))
        .min(b.distance(b.clamp(low, high)));
    for corner in [
        low,
        high,
        Vec2::new(low.x, high.y),
        Vec2::new(high.x, low.y),
    ] {
        let t = if delta.length_squared() > 0. {
            ((corner - a).dot(delta) / delta.length_squared()).clamp(0., 1.)
        } else {
            0.
        };
        distance = distance.min(corner.distance(a + delta * t));
    }
    distance
}

fn prepare_world(world: &mut World, camera: Vec2) -> Entity {
    for entity in world
        .query_filtered::<Entity, Or<(With<SessionEntity>, With<Sprite>, With<Node>, With<Mesh2d>)>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
    }
    let mut grid = IntentGrid::new(8, 8);
    for cell in [IVec2::ZERO, IVec2::X] {
        grid.paint(cell, IntentKind::Gather, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Build, SwarmId::PLAYER);
    }
    let reachability = SwarmReachabilityMap::new(&grid, [], [(SwarmId::PLAYER, Vec2::ZERO)])
        .expect("the traffic fixture has open terrain around the player's home");
    world.insert_resource(grid);
    world.insert_resource(reachability);
    for (mut transform, mut projection, mut zoom) in world
        .query::<(&mut Transform, &mut Projection, &mut CameraZoom2d)>()
        .iter_mut(world)
    {
        transform.translation.x = camera.x;
        transform.translation.y = camera.y;
        zoom.zoom = 1.1;
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scale = 1.1;
        }
    }
    world
        .spawn((Swarm {}, SwarmId::PLAYER, Transform::default()))
        .id()
}

fn prepare(world: &mut World) {
    let swarm = prepare_world(world, Vec2::new(504., 288.));
    let wall_scale = Vec3::new(4.5, 1.125, 1.);
    let source = world
        .spawn((
            Stockpile {
                kind: ResourceKind::Minerals,
                amount: 100,
                capacity: 100,
                radius: 36.,
            },
            StockpileRole::Source,
            OwnerSwarm(swarm),
            Structure::new(StructureKind::Basic),
            Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.)),
            Transform::from_xyz(432., 252., GAMEPLAY_SPRITE_Z).with_scale(wall_scale),
        ))
        .id();
    world.spawn((
        Structure::new(StructureKind::Basic),
        Sprite::from_color(Color::srgb(0.55, 0.35, 0.2), Vec2::splat(64.)),
        Transform::from_xyz(432., 396., GAMEPLAY_SPRITE_Z).with_scale(wall_scale),
    ));
    world.resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        100 + 2 * SHIPMENT,
    );
    let deliveries = [(340., 684., 1.), (524., 180., -1.)].map(|(start, end, direction)| {
        let facility = world
            .spawn((
                ProductionFacility::new(),
                Structure::new(StructureKind::Basic),
                OwnerSwarm(swarm),
                Sprite::from_color(Color::srgb(0.2, 0.42, 0.6), Vec2::splat(64.)),
                Transform::from_xyz(end, 252., GAMEPLAY_SPRITE_Z)
                    .with_scale(Vec3::new(1.125, 1.125, 1.)),
            ))
            .id();
        Delivery {
            bot: spawn_hauler(world, Vec2::new(start, 324.)),
            facility,
            direction,
        }
    });
    let loading = spawn_hauler(world, LOADING_POSITION);
    let previous = deliveries.each_ref().map(|delivery| {
        world
            .get::<Transform>(delivery.bot)
            .unwrap()
            .translation
            .truncate()
    });
    world.insert_resource(TrafficScene {
        deliveries,
        source,
        loading,
        previous,
        phase: Phase::Before,
        ticks: 0,
    });
    world.resource_mut::<Time<Virtual>>().pause();
    info!(
        "TRAFFIC before: two opposing deliveries and one loading position; paint corridor -3 -3 to start"
    );
}

fn spawn_hauler(world: &mut World, position: Vec2) -> Entity {
    world
        .spawn((
            Nanobot {},
            NanobotType::Hauler,
            Commitment::Working,
            Health::default(),
            VelocityComponent::default(),
            SwarmMember(SwarmId::PLAYER),
            Transform::from_translation(position.extend(GAMEPLAY_SPRITE_Z + 1.)),
        ))
        .id()
}

#[cfg(test)]
mod tests {
    use super::segment_rectangle_distance;
    use bevy::prelude::Vec2;

    #[test]
    fn crossing_segment_has_zero_rectangle_clearance() {
        let distance =
            segment_rectangle_distance(Vec2::new(-200., 0.), Vec2::new(200., 0.), Vec2::ZERO);
        assert!(distance.abs() < 0.0001, "crossing clearance: {distance}");
    }

    #[test]
    fn stationary_inside_has_zero_rectangle_clearance() {
        let distance =
            segment_rectangle_distance(Vec2::new(12., 5.), Vec2::new(12., 5.), Vec2::ZERO);
        assert!(distance.abs() < 0.0001, "inside clearance: {distance}");
    }

    #[test]
    fn stationary_outside_measures_nearest_corner() {
        let distance =
            segment_rectangle_distance(Vec2::new(180., 84.), Vec2::new(180., 84.), Vec2::ZERO);
        assert!(
            (distance - 60.).abs() < 0.0001,
            "outside corner clearance: {distance}"
        );
    }

    #[test]
    fn horizontal_parallel_segment_measures_vertical_gap() {
        let distance =
            segment_rectangle_distance(Vec2::new(-200., 100.), Vec2::new(200., 100.), Vec2::ZERO);
        assert!(
            (distance - 64.).abs() < 0.0001,
            "horizontal clearance: {distance}"
        );
    }

    #[test]
    fn vertical_parallel_segment_measures_horizontal_gap() {
        let distance =
            segment_rectangle_distance(Vec2::new(200., -100.), Vec2::new(200., 100.), Vec2::ZERO);
        assert!(
            (distance - 56.).abs() < 0.0001,
            "vertical clearance: {distance}"
        );
    }

    #[test]
    fn diagonal_segment_measures_interior_point_nearest_corner() {
        let distance =
            segment_rectangle_distance(Vec2::new(144., 46.), Vec2::new(154., 36.), Vec2::ZERO);
        assert!(
            (distance - 7.071_068).abs() < 0.0001,
            "diagonal corner clearance: {distance}"
        );
    }
}
