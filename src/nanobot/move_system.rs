use bevy::{
    prelude::{Commands, Entity, Quat, Query, Res, Transform, Vec2, Vec3, With},
    time::Time,
};

use crate::{
    game_settings::GameSettings,
    nanobot::consts::{BOT_RADIUS, BOT_SEPARATION_FORCE},
    spatial::FixedSpatialBuckets,
};

use super::{
    components::{DirectMovementComponent, Nanobot, ProgressChecker, VelocityComponent},
    consts::STOP_THRESHOLD,
};

pub fn move_velocity_system(
    time: Res<Time>,
    mut commands: Commands,
    mut bots: Query<(
        Entity,
        &DirectMovementComponent,
        &Transform,
        &mut VelocityComponent,
        Option<&mut ProgressChecker>,
    )>,
    game_settings: Res<GameSettings>,
) {
    let speed = game_settings.bot_speed;
    for (entity, bot_destination, transform, mut velocity, progress_checker) in bots.iter_mut() {
        let dest: Vec3 = [bot_destination.xy.x, bot_destination.xy.y, 0.].into();
        let translation = transform.translation;
        let direction = dest - translation;

        // The single stop authority: when the destination
        // carries an extent (`stop_radius > 0.0`), stop on
        // contact with the destination's physical edge
        // (clamped to the global `STOP_THRESHOLD` so very
        // small extents still resolve cleanly). When the
        // destination is extent-less (corridor waypoint,
        // legacy `Move` action, Defend cell center) the
        // `0.0` sentinel falls through to `STOP_THRESHOLD`,
        // matching the pre-issue behaviour for those paths.
        let stop_threshold = if bot_destination.stop_radius > 0.0 {
            bot_destination.stop_radius.max(STOP_THRESHOLD)
        } else {
            STOP_THRESHOLD
        };

        // Check if the distance is less than the threshold
        let distance = dest.distance(translation);
        if distance > stop_threshold {
            let new_velocity = direction.normalize() * speed.min(distance);
            velocity.value += new_velocity.truncate();

            // If the bot is not already moving, add a ProgressChecker
            if progress_checker.is_none() {
                commands.entity(entity).insert(ProgressChecker {
                    last_position: translation.truncate(),
                    last_update_time: time.elapsed_secs_f64(),
                });
            }
        } else {
            commands.entity(entity).remove::<DirectMovementComponent>();
            commands.entity(entity).remove::<ProgressChecker>();
        }

        // Check if the bot has not made any significant progress for a long time
        if let Some(mut checker) = progress_checker {
            let current_time = time.elapsed_secs_f64();
            const MAX_TIME_WITHOUT_PROGRESS: f64 = 2.0; // Maximum time without significant progress
            const MIN_PROGRESS: f32 = 1.0; // Minimum progress to reset the timer

            let progress = (checker.last_position - translation.truncate()).length();
            if progress < MIN_PROGRESS
                && current_time - checker.last_update_time > MAX_TIME_WITHOUT_PROGRESS
            {
                // The bot has not made significant progress for a long time, remove the destination
                commands.entity(entity).remove::<DirectMovementComponent>();
                commands.entity(entity).remove::<ProgressChecker>();
            } else if progress >= MIN_PROGRESS {
                // The bot has made significant progress, update the checker
                checker.last_position = translation.truncate();
                checker.last_update_time = current_time;
            }
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

pub fn rotation_for_direction(direction: Vec2) -> Option<Quat> {
    if direction.length_squared() <= MIN_FACING_SPEED * MIN_FACING_SPEED {
        return None;
    }
    let normalized = direction.normalize();
    Some(Quat::from_rotation_z(-normalized.x.atan2(normalized.y)))
}

pub fn velocity_system(mut query: Query<(&mut VelocityComponent, &mut Transform)>) {
    for (mut velocity, mut transform) in query.iter_mut() {
        transform.translation += velocity.value.extend(0.);
        if let Some(rotation) = rotation_for_direction(velocity.value) {
            transform.rotation = rotation;
        }
        velocity.value = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use bevy::prelude::EulerRot;

    use super::*;

    fn rotation_z(direction: Vec2) -> f32 {
        let rotation = rotation_for_direction(direction).expect("moving direction should rotate");
        let (_, _, z) = rotation.to_euler(EulerRot::XYZ);
        z
    }

    fn assert_angle_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected}, got {actual}"
        );
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
        assert_eq!(first_delta, -Vec2::X * BOT_SEPARATION_FORCE);
        assert_eq!(second_delta, Vec2::X * BOT_SEPARATION_FORCE);
        assert_eq!(distant_delta, Vec2::ZERO);
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
