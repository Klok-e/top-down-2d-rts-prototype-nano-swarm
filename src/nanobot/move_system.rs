use bevy::prelude::{Commands, Entity, Local, Quat, Query, Res, Transform, Vec2, With};

use crate::{
    game_settings::GameSettings,
    nanobot::consts::{BOT_RADIUS, BOT_SEPARATION_FORCE},
    spatial::FixedSpatialBuckets,
};

use super::{
    components::{DirectMovementComponent, Nanobot, ProgressChecker, VelocityComponent},
    consts::STOP_THRESHOLD,
};

pub struct ActiveRoute {
    destination: Vec2,
    stop_radius: f32,
    interaction: Option<super::InteractionRegion>,
    waypoints: Vec<Vec2>,
    current: usize,
    revision: u64,
}

#[allow(clippy::type_complexity)]
pub fn move_velocity_system(
    mut commands: Commands,
    mut routes: Local<std::collections::HashMap<Entity, ActiveRoute>>,
    mut bots: Query<(
        Entity,
        &DirectMovementComponent,
        &Transform,
        &mut VelocityComponent,
        Option<&super::NanobotType>,
        Option<&super::SwarmMember>,
    )>,
    game_settings: Res<GameSettings>,
    navigation: Res<crate::navigation::Navigation>,
    grid: Res<crate::intent::IntentGrid>,
) {
    use crate::navigation::RouteOutcome;
    routes.retain(|entity, _| bots.contains(*entity));
    for (entity, destination, transform, mut velocity, kind, member) in &mut bots {
        let position = transform.translation.truncate();
        let stop = destination.stop_radius.max(STOP_THRESHOLD);
        if destination
            .interaction
            .map_or(position.distance(destination.xy) <= stop, |region| {
                region.contains(position)
            })
            && navigation.point_clear(position)
        {
            commands
                .entity(entity)
                .remove::<DirectMovementComponent>()
                .remove::<ProgressChecker>();
            routes.remove(&entity);
            continue;
        }
        let needs_route = routes.get(&entity).is_none_or(|route| {
            route.destination != destination.xy
                || route.stop_radius != stop
                || route.interaction != destination.interaction
                || (route.waypoints.is_empty() && route.revision != navigation.revision())
                || route
                    .waypoints
                    .get(route.current)
                    .is_some_and(|next| !navigation.segment_clear(position, *next))
        });
        if needs_route {
            let swarm = member.map_or(super::SwarmId::PLAYER, |member| member.0);
            let hauler = kind == Some(&super::NanobotType::Hauler);
            let outcome = if let Some(region) = destination.interaction {
                navigation.route_to_interaction(position, region, &grid, swarm, hauler)
            } else if stop > STOP_THRESHOLD {
                navigation.route_within_range(position, destination.xy, stop, &grid, swarm, hauler)
            } else {
                navigation.route(position, destination.xy, &grid, swarm, hauler)
            };
            let waypoints = match outcome {
                RouteOutcome::Found(route) => route.waypoints,
                RouteOutcome::Unreachable => Vec::new(),
            };
            routes.insert(
                entity,
                ActiveRoute {
                    destination: destination.xy,
                    stop_radius: stop,
                    waypoints,
                    interaction: destination.interaction,
                    current: 0,
                    revision: navigation.revision(),
                },
            );
        }
        let route = routes.get_mut(&entity).expect("route was resolved");
        while route
            .waypoints
            .get(route.current)
            .is_some_and(|point| position.distance(*point) < 0.01)
        {
            route.current += 1;
        }
        if let Some(next) = route.waypoints.get(route.current) {
            let delta = *next - position;
            let speed = destination.speed.unwrap_or(game_settings.bot_speed);
            velocity.value += delta.normalize_or_zero() * speed.min(delta.length());
        }
    }
}

#[derive(Clone, Copy)]
struct SeparationEntry {
    entity: Entity,
    position: Vec2,
}

fn coincident_pair_direction(first: Entity, second: Entity) -> Vec2 {
    let mixed =
        first.to_bits().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ second.to_bits().rotate_left(32);
    let angle = (mixed as f64 / u64::MAX as f64 * std::f64::consts::TAU) as f32;
    Vec2::new(angle.cos(), angle.sin())
}

fn separation_deltas(entries: &[SeparationEntry]) -> Vec<(Entity, Vec2)> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| entry.entity.to_bits());

    let mut buckets = FixedSpatialBuckets::new(BOT_RADIUS * 2.0);
    for entry in &sorted {
        buckets.insert(entry.position, *entry);
    }
    buckets.sort_entries_by(|left, right| left.entity.to_bits().cmp(&right.entity.to_bits()));

    let mut deltas: Vec<(Entity, Vec2)> = sorted
        .iter()
        .map(|entry| (entry.entity, Vec2::ZERO))
        .collect();
    for entry in &sorted {
        let bucket = buckets.bucket_for_position(entry.position);
        for (_, neighbours) in buckets.neighbourhood(bucket, 1) {
            for other in neighbours {
                if other.entity.to_bits() <= entry.entity.to_bits() {
                    continue;
                }
                let offset = entry.position - other.position;
                if offset.length_squared() >= (BOT_RADIUS * 2.0).powi(2) {
                    continue;
                }
                let direction = if offset.length_squared() < 1e-6 {
                    coincident_pair_direction(entry.entity, other.entity)
                } else {
                    offset.normalize()
                };
                let force = direction * BOT_SEPARATION_FORCE;
                let first = deltas
                    .binary_search_by_key(&entry.entity.to_bits(), |(entity, _)| entity.to_bits())
                    .expect("snapshot entity must have a delta");
                let second = deltas
                    .binary_search_by_key(&other.entity.to_bits(), |(entity, _)| entity.to_bits())
                    .expect("neighbour entity must have a delta");
                deltas[first].1 += force;
                deltas[second].1 -= force;
            }
        }
    }
    deltas
}

pub fn separation_system(
    snapshots: Query<(Entity, &Transform), With<Nanobot>>,
    mut velocities: Query<&mut VelocityComponent, With<Nanobot>>,
) {
    let entries: Vec<_> = snapshots
        .iter()
        .map(|(entity, transform)| SeparationEntry {
            entity,
            position: transform.translation.truncate(),
        })
        .collect();

    for (entity, delta) in separation_deltas(&entries) {
        if let Ok(mut velocity) = velocities.get_mut(entity) {
            velocity.value += delta;
        }
    }
}

pub const MIN_FACING_SPEED: f32 = 0.001;

pub fn clamp_velocity(velocity: Vec2, max_speed: f32) -> Vec2 {
    if !velocity.is_finite() || !max_speed.is_finite() || max_speed <= 0.0 {
        return Vec2::ZERO;
    }
    let length = velocity.length();
    if !length.is_finite() {
        return Vec2::ZERO;
    }
    if length > max_speed {
        velocity / length * max_speed
    } else {
        velocity
    }
}

pub fn rotation_for_direction(direction: Vec2) -> Option<Quat> {
    if direction.length_squared() <= MIN_FACING_SPEED * MIN_FACING_SPEED {
        return None;
    }
    let normalized = direction.normalize();
    Some(Quat::from_rotation_z(-normalized.x.atan2(normalized.y)))
}

pub fn velocity_system(
    mut query: Query<(&mut VelocityComponent, &mut Transform)>,
    game_settings: Res<GameSettings>,
    navigation: Res<crate::navigation::Navigation>,
) {
    for (mut velocity, mut transform) in query.iter_mut() {
        let proposed = clamp_velocity(velocity.value, game_settings.bot_speed);
        let start = transform.translation.truncate();
        let applied_velocity = if navigation.segment_clear(start, start + proposed) {
            proposed
        } else {
            Vec2::ZERO
        };
        transform.translation += applied_velocity.extend(0.);
        if let Some(rotation) = rotation_for_direction(applied_velocity) {
            transform.rotation = rotation;
        }
        velocity.value = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use approx::assert_abs_diff_eq;
    use bevy::prelude::EulerRot;

    use super::*;

    fn rotation_z(direction: Vec2) -> f32 {
        let rotation = rotation_for_direction(direction).expect("moving direction should rotate");
        let (_, _, z) = rotation.to_euler(EulerRot::XYZ);
        z
    }

    fn assert_angle_close(actual: f32, expected: f32) {
        assert_abs_diff_eq!(actual, expected, epsilon = 0.0001);
    }

    #[test]
    fn facing_rotation_keeps_plus_y_as_unrotated_sprite_forward() {
        assert_angle_close(rotation_z(Vec2::Y), 0.0);
    }

    #[test]
    fn facing_rotation_turns_right_for_plus_x_motion() {
        assert_angle_close(rotation_z(Vec2::X), -FRAC_PI_2);
    }

    #[test]
    fn facing_rotation_turns_around_for_negative_y_motion() {
        assert_angle_close(rotation_z(Vec2::NEG_Y).abs(), PI);
    }

    #[test]
    fn facing_rotation_ignores_near_zero_motion() {
        assert!(rotation_for_direction(Vec2::ZERO).is_none());
    }

    #[test]
    fn velocity_clamp_zeroes_non_finite_input() {
        for clamped in [
            clamp_velocity(Vec2::new(f32::NAN, 1.0), 5.0),
            clamp_velocity(Vec2::new(f32::INFINITY, 0.0), 5.0),
        ] {
            assert_abs_diff_eq!(clamped.x, 0.0, epsilon = 1e-5);
            assert_abs_diff_eq!(clamped.y, 0.0, epsilon = 1e-5);
        }
    }

    #[test]
    fn velocity_clamp_preserves_direction_and_caps_length() {
        let clamped = clamp_velocity(Vec2::new(6.0, 8.0), 5.0);
        assert!((clamped.length() - 5.0).abs() < 1e-6);
        assert!((clamped - Vec2::new(3.0, 4.0)).length() < 1e-6);
    }

    fn entity(bits: u64) -> Entity {
        Entity::from_bits(bits)
    }

    #[test]
    fn separation_only_pushes_nearby_pairs_once() {
        let first = entity(1);
        let second = entity(2);
        let distant = entity(3);
        let deltas = separation_deltas(&[
            SeparationEntry {
                entity: first,
                position: Vec2::ZERO,
            },
            SeparationEntry {
                entity: second,
                position: Vec2::X,
            },
            SeparationEntry {
                entity: distant,
                position: Vec2::splat(BOT_RADIUS * 10.0),
            },
        ]);

        let first_delta = deltas.iter().find(|(id, _)| *id == first).unwrap().1;
        let second_delta = deltas.iter().find(|(id, _)| *id == second).unwrap().1;
        let distant_delta = deltas.iter().find(|(id, _)| *id == distant).unwrap().1;
        assert_abs_diff_eq!(first_delta.x, -BOT_SEPARATION_FORCE, epsilon = 1e-5);
        assert_abs_diff_eq!(first_delta.y, 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(second_delta.x, BOT_SEPARATION_FORCE, epsilon = 1e-5);
        assert_abs_diff_eq!(second_delta.y, 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(distant_delta.x, 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(distant_delta.y, 0.0, epsilon = 1e-5);
    }

    #[test]
    fn coincident_pair_separation_is_deterministic_and_finite() {
        let entries = [
            SeparationEntry {
                entity: entity(1),
                position: Vec2::ZERO,
            },
            SeparationEntry {
                entity: entity(2),
                position: Vec2::ZERO,
            },
        ];
        let first = separation_deltas(&entries);
        let second = separation_deltas(&entries);

        assert_eq!(first, second);
        assert!(first.iter().all(|(_, delta)| delta.is_finite()));
        assert!((first[0].1 + first[1].1).length_squared() < 1e-6);
    }
}
