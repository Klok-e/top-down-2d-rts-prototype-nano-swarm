//! Real-process headless acceptance fixture for opposing bottleneck traffic.
//!
//! Run with a private XDG_RUNTIME_DIR and `BEVY_ASSET_ROOT="$PWD"`:
//! `cargo run --example local_avoidance`.
//! Use `scripts/nano_swarm_control.py screenshot --name before`, then
//! `paint corridor -3 -3`. On the `TRAFFIC retreat` log, capture `retreat`,
//! then `paint corridor -2 -3`. On `TRAFFIC arrived`, capture `arrived`, then
//! `shutdown`. `wait --frames 2` synchronizes rendering while phases are paused.
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    GAMEPLAY_SPRITE_Z,
    fly_camera::CameraZoom2d,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, DirectMovementComponent, Health, Nanobot, NanobotType,
        OpponentIntentController, Structure, StructureKind, Swarm, SwarmId, SwarmMember,
        VelocityComponent,
    },
};

use top_down_2d_rts_prototype_nano_swarm::runtime::{RuntimeOptions, build_runtime_app};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Before,
    Approaching,
    Retreat,
    Resuming,
    Arrived,
}

#[derive(Resource)]
struct TrafficScene {
    bots: Vec<(Entity, Vec2)>,
    previous: Vec<Vec2>,
    phase: Phase,
    ticks: u32,
}

fn main() {
    let mut app = build_runtime_app(RuntimeOptions {
        headless: true,
        agent_socket: true,
        width: 1280,
        height: 720,
        ..Default::default()
    })
    .expect("headless runtime");
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1. / 60.),
    ));
    app.add_systems(PostStartup, prepare)
        .add_systems(First, release_phase)
        .add_systems(FixedPostUpdate, verify_tick);
    app.run();
}

fn release_phase(world: &mut World) {
    let Some(scene) = world.get_resource::<TrafficScene>() else {
        return;
    };
    let phase = scene.phase;
    let (cell, next) = match phase {
        Phase::Before => (IVec2::new(-3, -3), Phase::Approaching),
        Phase::Retreat => (IVec2::new(-2, -3), Phase::Resuming),
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
        let bots = world.resource::<TrafficScene>().bots.clone();
        for (entity, xy) in bots {
            world.entity_mut(entity).insert(DirectMovementComponent {
                xy,
                stop_radius: 0.,
                interaction: None,
                speed: None,
            });
        }
    }
    world.resource_mut::<TrafficScene>().phase = next;
    world.resource_mut::<Time<Virtual>>().unpause();
    info!("TRAFFIC running phase {next:?}");
}

fn verify_tick(
    mut scene: ResMut<TrafficScene>,
    positions: Query<&Transform>,
    mut time: ResMut<Time<Virtual>>,
) {
    let current: Vec<Vec2> = scene
        .bots
        .iter()
        .map(|(entity, _)| positions.get(*entity).unwrap().translation.truncate())
        .collect();
    for (i, &end) in current.iter().enumerate() {
        let start = scene.previous[i];
        for j in i + 1..current.len() {
            let separation = start - scene.previous[j];
            let change = end - current[j] - separation;
            let t = if change.length_squared() > 0. {
                (-separation.dot(change) / change.length_squared()).clamp(0., 1.)
            } else {
                0.
            };
            assert!(
                (separation + change * t).length() >= 67.99,
                "swept body overlap at tick {}: {current:?}",
                scene.ticks
            );
        }
        for center in [Vec2::new(432., 252.), Vec2::new(432., 396.)] {
            assert!(
                segment_rectangle_distance(start, end, center) >= 33.99,
                "swept structure collision at tick {}: {start:?} -> {end:?}",
                scene.ticks
            );
        }
    }
    scene.previous = current.clone();
    if !matches!(scene.phase, Phase::Approaching | Phase::Resuming) {
        return;
    }
    scene.ticks += 1;
    assert!(
        scene.ticks < 2400,
        "retained routes did not finish: {current:?}"
    );
    if scene.phase == Phase::Approaching
        && current
            .iter()
            .any(|position| (position.y - 324.).abs() > 50.)
    {
        scene.phase = Phase::Retreat;
        time.pause();
        info!(
            "TRAFFIC retreat: tick {}, positions {current:?}; paint corridor -2 -3 to resume",
            scene.ticks
        );
    } else if scene.phase == Phase::Resuming
        && current
            .iter()
            .zip(&scene.bots)
            .all(|(position, (_, goal))| position.distance(*goal) < 3.)
    {
        scene.phase = Phase::Arrived;
        time.pause();
        info!(
            "TRAFFIC arrived: tick {}, positions {current:?}; swept separation and structure clearance passed every fixed tick",
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

fn prepare(world: &mut World) {
    for entity in world
        .query_filtered::<Entity, Or<(With<Nanobot>, With<Sprite>, With<Swarm>)>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
    }
    for entity in world
        .query_filtered::<Entity, Or<(With<Node>, With<Mesh2d>)>>()
        .iter(world)
        .collect::<Vec<_>>()
    {
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
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
    let previous = bots
        .iter()
        .map(|(entity, _)| {
            world
                .get::<Transform>(*entity)
                .unwrap()
                .translation
                .truncate()
        })
        .collect();
    world.insert_resource(TrafficScene {
        bots,
        previous,
        phase: Phase::Before,
        ticks: 0,
    });
    world.resource_mut::<Time<Virtual>>().pause();
    info!("TRAFFIC before: four bodies; paint corridor -3 -3 to start");
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
