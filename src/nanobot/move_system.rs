use bevy::{ecs::system::SystemParam, prelude::*};

use super::{DirectMovementComponent, Nanobot, SwarmId, SwarmMember, VelocityComponent};
use crate::{
    game_settings::GameSettings, nanobot::consts::BOT_SEPARATION_FORCE,
    spatial::FixedSpatialBuckets,
};

/// A stalled traveller may temporarily pass through friendly bodies, never world geometry.
#[derive(Component, Debug)]
pub struct CongestionRecovery;

#[derive(Default)]
pub struct TravelProgress {
    goal: Vec2,
    best_distance: f32,
    last_progress: f64,
    recovering: bool,
    crossed: bool,
}

type ActiveWork = Or<(
    With<super::ExtractProgress>,
    With<super::HaulerLoading>,
    With<super::BuildProgress>,
    With<super::PlannedStructureProgress>,
    With<super::MaintenanceProgress>,
    With<super::ChargerProgress>,
)>;

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

    let mut buckets = FixedSpatialBuckets::new(crate::navigation::BODY_RADIUS * 2.0);
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
                if offset.length_squared() >= (crate::navigation::BODY_RADIUS * 2.0).powi(2) {
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

#[allow(clippy::type_complexity)]
pub fn separation_system(
    snapshots: Query<(Entity, &Transform), With<Nanobot>>,
    mut velocities: Query<
        (
            &mut VelocityComponent,
            Has<CongestionRecovery>,
            Has<super::RemainingTravel>,
            Has<super::WaitingForWork>,
        ),
        With<Nanobot>,
    >,
) {
    let entries: Vec<_> = snapshots
        .iter()
        .map(|(entity, transform)| SeparationEntry {
            entity,
            position: transform.translation.truncate(),
        })
        .collect();

    for (entity, delta) in separation_deltas(&entries) {
        if let Ok((mut velocity, recovering, travelling, queued)) = velocities.get_mut(entity)
            && (!recovering || !travelling || queued)
        {
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

#[derive(Clone, Copy)]
struct TrafficBody {
    entity: Entity,
    start: Vec2,
    delta: Vec2,
}

#[derive(Component, Debug)]
pub struct TrafficYield {
    pub(super) rejoin: Option<Vec2>,
}

#[derive(Debug)]
pub struct Yielding {
    to: Entity,
    retreat: Vec2,
    // Retracing local motion rejoins the retained route without a crowd detour.
    trail: Vec<Vec2>,
    returning: bool,
    side: Option<Vec2>,
}

fn swept_separation(start: Vec2, delta: Vec2, other: TrafficBody) -> f32 {
    let relative = start - other.start;
    let travel = delta - other.delta;
    let t = if travel.length_squared() > 1e-8 {
        (-relative.dot(travel) / travel.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (relative + travel * t).length()
}

#[derive(SystemParam)]
pub struct TrafficContext<'w, 's> {
    time: Res<'w, Time<Fixed>>,
    members: Query<'w, 's, &'static SwarmMember>,
    remaining_travel: Query<'w, 's, &'static super::RemainingTravel>,
    active_work: Query<'w, 's, (), ActiveWork>,
    commitments: Query<'w, 's, &'static super::Commitment>,
    queued: Query<'w, 's, (), With<super::WaitingForWork>>,
}

#[allow(clippy::type_complexity)]
pub fn velocity_system(
    mut commands: Commands,
    mut world: ParamSet<(
        crate::physical_world::PhysicalWorld,
        Query<(
            Entity,
            &mut VelocityComponent,
            &mut Transform,
            Option<&Nanobot>,
            Option<&TrafficYield>,
            Option<&DirectMovementComponent>,
        )>,
    )>,
    mut yielding: Local<std::collections::HashMap<Entity, Yielding>>,
    mut progress: Local<std::collections::HashMap<Entity, TravelProgress>>,
    traffic: TrafficContext,
    game_settings: Res<GameSettings>,
    navigation: Res<crate::navigation::Navigation>,
) {
    let TrafficContext {
        time,
        members,
        remaining_travel,
        active_work,
        commitments,
        queued,
    } = traffic;
    let physical = world.p0().snapshot();
    let mut query = world.p1();
    let diameter = crate::navigation::BODY_RADIUS * 2.0;
    let mut bodies: Vec<_> = query
        .iter()
        .filter(|(_, _, _, bot, _, _)| bot.is_some())
        .map(|(entity, velocity, transform, _, _, _)| {
            (
                TrafficBody {
                    entity,
                    start: transform.translation.truncate(),
                    delta: Vec2::ZERO,
                },
                clamp_velocity(velocity.value, game_settings.bot_speed),
            )
        })
        .collect();
    bodies.sort_by_key(|(body, _)| body.entity.to_bits());
    let mut lookup = FixedSpatialBuckets::new(diameter * 2.0 + game_settings.bot_speed * 2.0);
    let indices: std::collections::HashMap<_, _> = bodies
        .iter()
        .enumerate()
        .map(|(i, (body, _))| (body.entity, i))
        .collect();
    for (i, (body, _)) in bodies.iter().enumerate() {
        lookup.insert(body.start, i);
    }
    let now = time.elapsed_secs_f64();
    progress.retain(|entity, _| indices.contains_key(entity));
    let friendly = |a: Entity, b: Entity| {
        members.get(a).map_or(SwarmId::PLAYER, |member| member.0)
            == members.get(b).map_or(SwarmId::PLAYER, |member| member.0)
    };
    let pinned: std::collections::HashSet<_> = bodies
        .iter()
        .filter_map(|(body, _)| {
            ((active_work.contains(body.entity)
                || commitments
                    .get(body.entity)
                    .is_ok_and(|value| *value == super::Commitment::Working))
                && query.get(body.entity).unwrap().5.is_none()
                && !queued.contains(body.entity))
            .then_some(body.entity)
        })
        .collect();
    for i in 0..bodies.len() {
        let (body, _) = bodies[i];
        let movement = query
            .get(body.entity)
            .unwrap()
            .5
            .map(|movement| movement.xy);
        if pinned.contains(&body.entity) {
            bodies[i].1 = Vec2::ZERO;
        } else if movement.is_none() || queued.contains(body.entity) {
            // Step sideways before an approaching friendly reaches the waiting body's space.
            let approaching = lookup
                .neighbourhood(lookup.bucket_for_position(body.start), 1)
                .flat_map(|(_, entries)| entries.iter().copied())
                .filter(|j| *j != i)
                .filter(|j| friendly(body.entity, bodies[*j].0.entity))
                .find(|j| {
                    let (other, wanted) = bodies[*j];
                    query.get(other.entity).unwrap().5.is_some()
                        && !queued.contains(other.entity)
                        && wanted.dot(body.start - other.start) > 0.0
                        && swept_separation(
                            body.start,
                            Vec2::ZERO,
                            TrafficBody {
                                delta: wanted.normalize_or_zero() * diameter * 2.0,
                                ..other
                            },
                        ) < diameter + 2.0
                });
            if let Some(j) = approaching {
                let heading = bodies[j].1.normalize_or_zero();
                let side = Vec2::new(-heading.y, heading.x);
                let offset = body.start - bodies[j].0.start;
                let preferred = if offset.dot(side) >= 0.0 { side } else { -side };
                bodies[i].1 = [preferred, -preferred]
                    .into_iter()
                    .find_map(|direction| {
                        let delta = direction * game_settings.bot_speed;
                        physical
                            .movement_clear(body.start, body.start + direction * diameter)
                            .then_some(delta)
                    })
                    .unwrap_or(Vec2::ZERO);
            }
        }
        let overlapping = lookup
            .neighbourhood(lookup.bucket_for_position(body.start), 1)
            .flat_map(|(_, entries)| entries.iter().copied())
            .any(|j| j != i && body.start.distance(bodies[j].0.start) < diameter - 0.001);
        let Some(movement) = movement.filter(|_| !queued.contains(body.entity)) else {
            let recovering_overlap = overlapping
                && progress
                    .get(&body.entity)
                    .is_some_and(|state| state.recovering);
            if recovering_overlap {
                progress.get_mut(&body.entity).unwrap().crossed = true;
            } else if progress.remove(&body.entity).is_some() {
                commands.entity(body.entity).remove::<CongestionRecovery>();
            }
            continue;
        };
        let Ok(remaining) = remaining_travel.get(body.entity) else {
            let recovering_overlap = overlapping
                && progress
                    .get(&body.entity)
                    .is_some_and(|state| state.recovering);
            if recovering_overlap {
                progress.get_mut(&body.entity).unwrap().crossed = true;
            } else if progress.remove(&body.entity).is_some() {
                commands.entity(body.entity).remove::<CongestionRecovery>();
            }
            continue;
        };
        let distance = remaining.0;
        let state = progress.entry(body.entity).or_insert(TravelProgress {
            goal: movement,
            best_distance: distance,
            last_progress: now,
            ..Default::default()
        });
        if state.goal != movement {
            state.goal = movement;
            state.best_distance = distance;
            state.last_progress = now;
        } else if distance + 2.0 < state.best_distance {
            state.best_distance = distance;
            state.last_progress = now;
        }
        state.crossed |= state.recovering && overlapping;
        if state.recovering && (state.crossed || state.last_progress == now) && !overlapping {
            state.recovering = false;
            state.crossed = false;
            state.best_distance = distance;
            state.last_progress = now;
            commands.entity(body.entity).remove::<CongestionRecovery>();
        }
    }
    yielding.retain(|entity, state| {
        let retain = indices.contains_key(entity) && indices.contains_key(&state.to);
        if !retain && indices.contains_key(entity) {
            commands.entity(*entity).remove::<TrafficYield>();
        }
        retain
    });
    // Earlier bodies reserve swept motion; later bodies remain stationary until resolved.
    for i in 0..bodies.len() {
        let (body, desired) = bodies[i];
        if pinned.contains(&body.entity) {
            yielding.remove(&body.entity);
            commands.entity(body.entity).remove::<TrafficYield>();
            continue;
        }
        let nearby: Vec<_> = lookup
            .neighbourhood(lookup.bucket_for_position(body.start), 1)
            .flat_map(|(_, entries)| entries.iter().copied())
            .filter(|j| *j != i)
            .collect();
        if let Some(state) = progress.get_mut(&body.entity) {
            let obstructed_by_friend = remaining_travel
                .get(body.entity)
                .is_ok_and(|remaining| remaining.0 > 0.01)
                && nearby.iter().any(|j| {
                    let other = bodies[*j].0;
                    friendly(body.entity, other.entity)
                        && swept_separation(body.start, desired, other) < diameter
                });
            if !state.recovering && now - state.last_progress >= 1.0 && obstructed_by_friend {
                state.recovering = true;
                state.crossed = false;
                commands.entity(body.entity).insert(CongestionRecovery);
            }
            if state.recovering {
                yielding.remove(&body.entity);
                commands.entity(body.entity).remove::<TrafficYield>();
            }
        }
        let recovering = progress
            .get(&body.entity)
            .is_some_and(|state| state.recovering);
        if let Some(state) = yielding.get_mut(&body.entity) {
            let (other, other_wanted) = bodies[indices[&state.to]];
            let following = nearby.iter().any(|j| {
                let (other, wanted) = bodies[*j];
                wanted.dot(state.retreat) > 0.01
                    && (other.start - body.start).dot(state.retreat) <= diameter
                    && other.start.distance(body.start) < diameter * 6.0
            });
            if !following
                && (other_wanted.dot(state.retreat) <= 0.01
                    || (other.start - body.start).dot(state.retreat) > diameter
                    || other.start.distance(body.start) > diameter * 6.0)
            {
                state.returning = true;
            }
        }
        if !recovering
            && !yielding.contains_key(&body.entity)
            && desired.length_squared() > 0.01
            && query.get(body.entity).unwrap().5.is_some()
        {
            let conflict = nearby.iter().copied().find(|j| {
                let (other, wanted) = bodies[*j];
                let toward = other.start - body.start;
                let heading = desired.normalize_or_zero();
                let other_heading = wanted.normalize_or_zero();
                let other_precedes = other_heading
                    .x
                    .total_cmp(&heading.x)
                    .then_with(|| other_heading.y.total_cmp(&heading.y))
                    .then_with(|| body.entity.to_bits().cmp(&other.entity.to_bits()))
                    .is_gt();
                other_precedes
                    && query.get(other.entity).unwrap().5.is_some()
                    && toward.length() >= diameter - 0.01
                    && toward.length() < diameter + game_settings.bot_speed * 4.0
                    && desired.dot(wanted) < 0.0
                    && toward.dot(desired) > 0.0
                    && swept_separation(
                        body.start,
                        desired.normalize_or_zero() * diameter,
                        TrafficBody {
                            delta: wanted.normalize_or_zero() * diameter,
                            ..other
                        },
                    ) < diameter
            });
            if let Some(j) = conflict {
                yielding.insert(
                    body.entity,
                    Yielding {
                        to: bodies[j].0.entity,
                        retreat: -desired.normalize(),
                        trail: vec![body.start],
                        returning: false,
                        side: None,
                    },
                );
                commands
                    .entity(body.entity)
                    .insert(TrafficYield { rejoin: None });
            }
        }
        if !recovering && !yielding.contains_key(&body.entity) {
            let inherited = nearby.iter().find_map(|j| {
                let other = bodies[*j].0;
                let state = yielding.get(&other.entity)?;
                let offset = body.start - other.start;
                (!state.returning
                    && body.entity != state.to
                    && desired.dot(bodies[indices[&state.to]].1) <= 0.0
                    && offset.length() < diameter + game_settings.bot_speed * 4.0
                    && offset.dot(state.retreat) > 0.0)
                    .then_some((state.to, state.retreat))
            });
            if let Some((to, retreat)) = inherited {
                yielding.insert(
                    body.entity,
                    Yielding {
                        to,
                        retreat,
                        trail: vec![body.start],
                        returning: false,
                        side: None,
                    },
                );
                commands
                    .entity(body.entity)
                    .insert(TrafficYield { rejoin: None });
            }
        }
        if yielding
            .get(&body.entity)
            .is_some_and(|state| state.returning)
            && query
                .get(body.entity)
                .ok()
                .and_then(|(_, _, _, _, marker, _)| marker.and_then(|marker| marker.rejoin))
                .is_some_and(|point| navigation.segment_clear(body.start, point))
        {
            yielding.remove(&body.entity);
            commands.entity(body.entity).remove::<TrafficYield>();
        }
        let safe = |delta: Vec2| {
            physical.movement_clear(body.start, body.start + delta)
                && nearby.iter().all(|j| {
                    let other = bodies[*j].0;
                    // Invalid initial overlaps may separate, but cannot deepen.
                    let passing_friend = friendly(body.entity, other.entity)
                        && (recovering
                            || progress
                                .get(&other.entity)
                                .is_some_and(|state| state.recovering));
                    let required = if passing_friend {
                        0.0
                    } else {
                        diameter.min(body.start.distance(other.start))
                    };
                    swept_separation(body.start, delta, other) + 0.001 >= required
                })
        };
        let speed = game_settings.bot_speed;
        let applied = if let Some(state) = yielding.get_mut(&body.entity) {
            while state.returning
                && state
                    .trail
                    .last()
                    .is_some_and(|point| point.distance(body.start) < 0.01)
            {
                state.trail.pop();
            }
            if state.returning {
                let delta = state.trail.last().map_or(Vec2::ZERO, |point| {
                    clamp_velocity(*point - body.start, speed)
                });
                if safe(delta) { delta } else { Vec2::ZERO }
            } else {
                let side = Vec2::new(-state.retreat.y, state.retreat.x);
                let choices = state.side.map_or(vec![side, -side, state.retreat], |side| {
                    vec![side, state.retreat]
                });
                choices
                    .into_iter()
                    .find_map(|direction| {
                        let delta = direction * speed;
                        if navigation
                            .segment_clear(body.start, body.start + direction * (diameter + 2.0))
                            && safe(delta)
                        {
                            if direction != state.retreat {
                                state.side = Some(direction);
                            }
                            Some(delta)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(Vec2::ZERO)
            }
        } else if safe(desired) {
            desired
        } else {
            let direction = desired.normalize_or_zero();
            let side = Vec2::new(-direction.y, direction.x);
            [
                (direction + side).normalize_or_zero(),
                (direction - side).normalize_or_zero(),
                side,
                -side,
            ]
            .into_iter()
            .find_map(|direction| {
                let delta = direction * desired.length();
                safe(delta).then_some(delta)
            })
            .unwrap_or(Vec2::ZERO)
        };
        if let Some(state) = yielding.get_mut(&body.entity) {
            if !state.returning && applied.length_squared() > 0.0 {
                state.trail.push(body.start + applied);
            }
            if state.returning && state.trail.is_empty() {
                yielding.remove(&body.entity);
                commands.entity(body.entity).remove::<TrafficYield>();
            }
        }
        bodies[i].0.delta = applied;
    }
    for (entity, mut velocity, mut transform, bot, _, _) in &mut query {
        let start = transform.translation.truncate();
        let applied_velocity = if bot.is_some() {
            bodies[indices[&entity]].0.delta
        } else {
            let proposed = clamp_velocity(velocity.value, game_settings.bot_speed);
            if physical.movement_clear(start, start + proposed) {
                proposed
            } else {
                Vec2::ZERO
            }
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
                position: Vec2::splat(crate::nanobot::consts::BOT_RADIUS * 10.0),
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
