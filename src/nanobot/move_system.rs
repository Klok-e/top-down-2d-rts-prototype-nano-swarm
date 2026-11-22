use bevy::{ecs::system::SystemParam, prelude::*};

use super::{
    CombatIdentity, DefenderEngagements, DirectMovementComponent, Nanobot, SwarmId, SwarmMember,
    VelocityComponent, combat_key, route_following::WAYPOINT_REACHED_DISTANCE,
};
use crate::{
    nanobot::consts::BOT_SEPARATION_FORCE,
    spatial::{FixedSpatialBuckets, PackedSpatialBuckets},
    tuning::Tuning,
};

pub(super) const BODY_DIAMETER: f32 = crate::navigation::BODY_RADIUS * 2.0;

/// A stalled traveller may temporarily pass through friendly bodies, never world geometry.
#[derive(Component, Debug)]
pub struct CongestionRecovery;

#[derive(Default)]
pub struct TravelProgress {
    goal: Vec2,
    region: Option<super::InteractionRegion>,
    best_distance: f32,
    last_progress: f64,
    recovering: bool,
    crossed: bool,
}

#[derive(Default)]
pub struct LocalMotion {
    goal: Option<(Vec2, Option<super::InteractionRegion>)>,
    velocity: Vec2,
    departure: Option<Vec2>,
    separation_remainder: Vec2,
}

type ActiveWork = Or<(
    With<super::ExtractProgress>,
    With<super::HaulerLoading>,
    With<super::BuildProgress>,
    With<super::PlannedStructureProgress>,
    With<super::MaintenanceProgress>,
    With<super::ChargerProgress>,
)>;

type AssignedWork = Or<(
    With<super::GatherAssignment>,
    With<super::ReturningToStockpile>,
    With<super::HaulerAssignment>,
    With<super::BuildAssignment>,
    With<super::PlannedStructureClaim>,
    With<super::MaintenanceAssignment>,
    With<super::ChargerAssignment>,
)>;

#[derive(Clone, Copy)]
struct SeparationEntry {
    entity: Entity,
    position: Vec2,
    key: u64,
}

pub(super) fn coincident_pair_direction(first: u64, second: u64) -> Vec2 {
    let mixed = first.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ second.rotate_left(32);
    let angle = (mixed as f64 / u64::MAX as f64 * std::f64::consts::TAU) as f32;
    Vec2::new(angle.cos(), angle.sin())
}

fn separation_deltas(entries: &[SeparationEntry]) -> Vec<(Entity, Vec2)> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| (entry.key, entry.entity.to_bits()));

    let mut buckets = FixedSpatialBuckets::new(BODY_DIAMETER);
    for (index, entry) in sorted.iter().enumerate() {
        buckets.insert(entry.position, index);
    }

    let mut deltas: Vec<(Entity, Vec2)> = sorted
        .iter()
        .map(|entry| (entry.entity, Vec2::ZERO))
        .collect();
    for (index, entry) in sorted.iter().enumerate() {
        let bucket = buckets.bucket_for_position(entry.position);
        for (_, neighbours) in buckets.neighbourhood(bucket, 1) {
            for &other_index in neighbours {
                if other_index <= index {
                    continue;
                }
                let other = sorted[other_index];
                let offset = entry.position - other.position;
                let squared = offset.length_squared();
                if squared >= BODY_DIAMETER.powi(2) {
                    continue;
                }
                let distance = squared.sqrt();
                let direction = if squared < 1e-6 {
                    coincident_pair_direction(entry.key, other.key)
                } else {
                    offset / distance
                };
                let force = direction
                    * (BOT_SEPARATION_FORCE * 2.0 * (BODY_DIAMETER - distance) / BODY_DIAMETER);
                deltas[index].1 += force;
                deltas[other_index].1 -= force;
            }
        }
    }
    deltas
}

#[allow(clippy::type_complexity)]
pub fn separation_system(
    snapshots: Query<(Entity, &Transform, Option<&CombatIdentity>), With<Nanobot>>,
    engagements: Option<Res<DefenderEngagements>>,
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
        .map(|(entity, transform, identity)| SeparationEntry {
            entity,
            position: transform.translation.truncate(),
            key: combat_key(entity, identity),
        })
        .collect();

    for (entity, delta) in separation_deltas(&entries) {
        let Ok((mut velocity, recovering, travelling, queued)) = velocities.get_mut(entity) else {
            continue;
        };
        let engaged = engagements
            .as_ref()
            .is_some_and(|state| state.is_engaged(entity));
        let push = if recovering && travelling && !queued && !engaged {
            Vec2::ZERO
        } else {
            delta
        };
        velocity.value = engagements.as_ref().map_or(velocity.value + push, |state| {
            state.velocity_with_separation(entity, velocity.value, push)
        });
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
    key: u64,
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

/// A body that a swept motion must keep `required` clearance from.
#[derive(Clone, Copy)]
struct SweepConstraint {
    other: TrafficBody,
    required: f32,
    /// The answer whenever the closest approach is the starting separation.
    clears_at_start: bool,
}

impl SweepConstraint {
    fn new(start: Vec2, other: TrafficBody, required: f32) -> Self {
        Self {
            other,
            required,
            clears_at_start: (start - other.start).length() + 0.001 >= required,
        }
    }

    /// Whether moving from `start` by `delta` keeps clearance within the solver's
    /// tolerance: `swept_separation(start, delta, other) + 0.001 >= required`.
    fn clears(&self, start: Vec2, delta: Vec2) -> bool {
        let relative = start - self.other.start;
        let travel = delta - self.other.delta;
        // Without closing motion the sweep projects to its start, which is precomputed.
        if travel.length_squared() <= 1e-8 || relative.dot(travel) >= 0.0 {
            return self.clears_at_start;
        }
        !self.ends_inside(start, delta)
            && swept_separation(start, delta, self.other) + 0.001 >= self.required
    }

    /// Whether the motion ends so deep inside the clearance that it cannot clear,
    /// whatever its path. The slack covers the solver tolerance and rounding.
    fn ends_inside(&self, start: Vec2, delta: Vec2) -> bool {
        let relative = start - self.other.start;
        let travel = delta - self.other.delta;
        let slack = 0.011 + 1e-4 * (relative.abs() + travel.abs()).element_sum();
        let reject = self.required - slack;
        reject > 0.0 && (relative + travel).length_squared() < reject * reject
    }
}

/// The previous velocity plus four rings of headings around it.
const CANDIDATE_COUNT: usize = 1 + 4 * 24;

/// Unit headings of the velocity candidates, evaluated once so every call shares them.
static CANDIDATE_HEADINGS: std::sync::LazyLock<[Vec2; 24]> = std::sync::LazyLock::new(|| {
    std::array::from_fn(|heading| {
        let angle = heading as f32 * std::f32::consts::TAU / 24.0;
        Vec2::new(angle.cos(), angle.sin())
    })
});

/// Choose a safe velocity in the acceleration-reachable disk, closest to the requested motion.
/// A blocked disk permits an immediate stop; collision clearance is authoritative.
fn continuous_velocity(
    previous: Vec2,
    requested: Vec2,
    acceleration: f32,
    speed_limit: f32,
    safe: impl Fn(Vec2) -> bool,
) -> Vec2 {
    if requested.length_squared() == 0.0 || acceleration <= 0.0 {
        return Vec2::ZERO;
    }
    let preferred = clamp_velocity(
        previous + clamp_velocity(requested - previous, acceleration),
        speed_limit,
    );
    if safe(preferred) {
        return preferred;
    }
    let requested_speed = requested.length();
    let requested_direction = requested / requested_speed;
    // Every heading of the innermost ring is the previous velocity itself, so it
    // contributes one candidate. Scoring all candidates in one pass lets it vectorize.
    let mut candidates = [Vec2::ZERO; CANDIDATE_COUNT];
    let scaled = CANDIDATE_HEADINGS.map(|heading| heading * acceleration);
    candidates[0] = previous + scaled[0] * 0.0 / 4.0;
    for (ring, candidates) in candidates[1..].chunks_exact_mut(scaled.len()).enumerate() {
        let ring = (ring + 1) as f32;
        for (candidate, scaled) in candidates.iter_mut().zip(&scaled) {
            *candidate = previous + *scaled * ring / 4.0;
        }
    }
    let mut scores = [0.0; CANDIDATE_COUNT];
    for (score, candidate) in scores.iter_mut().zip(&candidates) {
        let candidate = *candidate;
        let lateral = candidate.perp_dot(requested_direction);
        let lost_speed = requested_speed - candidate.length();
        // Retained momentum carries a passing body around contact instead of alternating tangents.
        let value = candidate.distance_squared(requested)
            + lateral * lateral
            + 2.0 * lost_speed * lost_speed
            + 0.5 * candidate.distance_squared(previous);
        // Candidates beyond the speed limit can never win.
        *score = if candidate.length_squared() > speed_limit * speed_limit + 1e-6 {
            f32::INFINITY
        } else {
            value
        };
    }
    let mut best = None;
    let mut best_score = f32::INFINITY;
    for (&candidate, &score) in candidates.iter().zip(&scores) {
        if score < best_score && safe(candidate) {
            best = Some(candidate);
            best_score = score;
        }
    }
    best.unwrap_or(Vec2::ZERO)
}

#[derive(SystemParam)]
pub struct TrafficContext<'w, 's> {
    motion: Local<'s, std::collections::HashMap<Entity, LocalMotion>>,
    previous_generation: Local<'s, Option<u64>>,
    generation: Option<Res<'w, crate::session_lifecycle::SessionGeneration>>,
    time: Res<'w, Time<Fixed>>,
    members: Query<'w, 's, &'static SwarmMember>,
    remaining_travel: Query<'w, 's, &'static super::RemainingTravel>,
    active_work: Query<'w, 's, (), ActiveWork>,
    assigned_work: Query<'w, 's, (), AssignedWork>,
    cargo: Query<'w, 's, &'static super::Cargo>,
    commitments: Query<'w, 's, &'static super::Commitment>,
    approaches: Query<'w, 's, (Entity, &'static super::WorkApproach)>,
    queued: Query<'w, 's, (), With<super::WaitingForWork>>,
    engagements: Option<Res<'w, DefenderEngagements>>,
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
            Option<&mut super::RegionalLease>,
            Option<&CombatIdentity>,
        )>,
    )>,
    mut yielding: Local<std::collections::HashMap<Entity, Yielding>>,
    mut progress: Local<std::collections::HashMap<Entity, TravelProgress>>,
    traffic: TrafficContext,
    tuning: Res<Tuning>,
    navigation: Res<crate::navigation::Navigation>,
) {
    let TrafficContext {
        mut motion,
        mut previous_generation,
        generation,
        time,
        members,
        remaining_travel,
        active_work,
        assigned_work,
        cargo,
        commitments,
        queued,
        approaches,
        engagements,
    } = traffic;
    if let Some(generation) = generation.as_deref().map(|generation| generation.0)
        && *previous_generation != Some(generation)
    {
        motion.clear();
        yielding.clear();
        progress.clear();
        *previous_generation = Some(generation);
    }
    let physical = world.p0().snapshot();
    let mut query = world.p1();
    let diameter = BODY_DIAMETER;
    let is_engaged = |entity| {
        engagements
            .as_ref()
            .is_some_and(|value| value.controls_movement(entity))
    };
    let holds_engagement = |entity| {
        engagements
            .as_ref()
            .is_some_and(|value| value.is_engaged(entity))
    };
    let move_order = |entity| {
        let (_, _, _, _, _, order, ..) = query.get(entity).ok()?;
        order
    };
    let mut bodies: Vec<_> = query
        .iter()
        .filter(|(_, _, _, bot, ..)| bot.is_some())
        .map(|(entity, velocity, transform, .., identity)| {
            (
                TrafficBody {
                    entity,
                    start: transform.translation.truncate(),
                    delta: Vec2::ZERO,
                    key: combat_key(entity, identity),
                },
                clamp_velocity(velocity.value, tuning.logistics.bot_speed),
            )
        })
        .collect();
    bodies.sort_by_key(|(body, _)| (body.key, body.entity.to_bits()));
    let indices: std::collections::HashMap<_, _> = bodies
        .iter()
        .enumerate()
        .map(|(i, (body, _))| (body.entity, i))
        .collect();
    let lookup = PackedSpatialBuckets::new(
        diameter * 2.0 + tuning.logistics.bot_speed * 2.0,
        bodies
            .iter()
            .enumerate()
            .map(|(i, (body, _))| (body.start, i)),
    );
    let now = time.elapsed_secs_f64();
    progress.retain(|entity, _| indices.contains_key(entity));
    motion.retain(|entity, _| indices.contains_key(entity));
    let friendly = |a: Entity, b: Entity| {
        members.get(a).map_or(SwarmId::PLAYER, |member| member.0)
            == members.get(b).map_or(SwarmId::PLAYER, |member| member.0)
    };
    let mut demanded_regions = Vec::new();
    for (entity, approach) in &approaches {
        let owner = members
            .get(entity)
            .map_or(SwarmId::PLAYER, |member| member.0);
        if !demanded_regions.contains(&(owner, approach.region)) {
            demanded_regions.push((owner, approach.region));
        }
    }
    let pinned: std::collections::HashSet<_> = bodies
        .iter()
        .filter_map(|(body, _)| {
            (!is_engaged(body.entity)
                && (active_work.contains(body.entity)
                    || assigned_work.contains(body.entity)
                    || cargo.get(body.entity).is_ok_and(|cargo| cargo.amount > 0)
                    || commitments
                        .get(body.entity)
                        .is_ok_and(|value| *value == super::Commitment::Working))
                && move_order(body.entity).is_none()
                && !queued.contains(body.entity))
            .then_some(body.entity)
        })
        .collect();
    for i in 0..bodies.len() {
        let (body, _) = bodies[i];
        if is_engaged(body.entity) {
            let state = motion.entry(body.entity).or_default();
            state.goal = None;
            state.departure = None;
            state.separation_remainder = Vec2::ZERO;
            if yielding.remove(&body.entity).is_some() {
                commands.entity(body.entity).remove::<TrafficYield>();
            }
            if progress.remove(&body.entity).is_some() {
                commands.entity(body.entity).remove::<CongestionRecovery>();
            }
            continue;
        }
        let order = move_order(body.entity);
        let movement = order.map(|order| order.xy);
        if pinned.contains(&body.entity) {
            bodies[i].1 = Vec2::ZERO;
            motion.entry(body.entity).or_default().departure = None;
        } else if movement.is_none() || queued.contains(body.entity) {
            // Step sideways before an approaching friendly reaches the waiting body's space.
            let approaching = lookup
                .neighbourhood(lookup.bucket_for_position(body.start), 1)
                .flat_map(|(_, entries)| entries.iter().copied())
                .filter(|j| *j != i)
                .filter(|j| friendly(body.entity, bodies[*j].0.entity))
                .find(|j| {
                    let (other, wanted) = bodies[*j];
                    move_order(other.entity).is_some()
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
                        let delta = direction * tuning.logistics.bot_speed;
                        physical
                            .movement_clear(body.start, body.start + direction * diameter)
                            .then_some(delta)
                    })
                    .unwrap_or(Vec2::ZERO);
            }
        }
        if movement.is_none() && !queued.contains(body.entity) && !pinned.contains(&body.entity) {
            if motion
                .entry(body.entity)
                .or_default()
                .departure
                .is_some_and(|target| !physical.movement_clear(body.start, target))
            {
                motion.entry(body.entity).or_default().departure = None;
            }
            if motion.entry(body.entity).or_default().departure.is_none() {
                let owner = members
                    .get(body.entity)
                    .map_or(SwarmId::PLAYER, |member| member.0);
                let demanded = demanded_regions.iter().find_map(|(swarm, region)| {
                    (*swarm == owner
                        && !approaches.contains(body.entity)
                        && body.start.distance(region.approach(body.start)) < diameter)
                        .then_some(*region)
                });
                if let Some(region) = demanded {
                    let target = (0..8)
                        .map(|step| {
                            let angle = step as f32 * std::f32::consts::FRAC_PI_4;
                            body.start + Vec2::new(angle.cos(), angle.sin()) * diameter
                        })
                        .filter(|point| physical.movement_clear(body.start, *point))
                        .max_by(|a, b| {
                            a.distance_squared(region.approach(*a))
                                .total_cmp(&b.distance_squared(region.approach(*b)))
                        });
                    if let Some(target) = target {
                        motion.entry(body.entity).or_default().departure = Some(target);
                    }
                }
            }
            if let Some(target) = motion.entry(body.entity).or_default().departure {
                if body.start.distance(target) <= 0.01 {
                    motion.entry(body.entity).or_default().departure = None;
                } else {
                    bodies[i].1 = clamp_velocity(target - body.start, tuning.logistics.bot_speed);
                }
            }
        } else {
            motion.entry(body.entity).or_default().departure = None;
        }
        let region = order.and_then(|order| order.interaction);
        let preference = motion.entry(body.entity).or_default();
        let goal = order.map(|order| (order.xy, order.interaction));
        let same_goal = match (preference.goal, goal) {
            (Some((_, Some(old))), Some((_, Some(new)))) => old == new,
            (old, new) => old == new,
        };
        if !same_goal {
            if yielding.remove(&body.entity).is_some() {
                commands.entity(body.entity).remove::<TrafficYield>();
            }
            preference.goal = goal;
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
        let distance = region.map_or(remaining.0, |region| {
            body.start.distance(region.approach(body.start))
        });
        let state = progress.entry(body.entity).or_insert(TravelProgress {
            goal: movement,
            region,
            best_distance: distance,
            last_progress: now,
            ..Default::default()
        });
        if state.region != region || (region.is_none() && state.goal != movement) {
            state.goal = movement;
            state.region = region;
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
    let speed = tuning.logistics.bot_speed;
    // Either body can spend a full step closing, with room for the solver's tolerance and rounding.
    let sweep_reach_squared = (diameter + 2.0 * speed + 0.01 + 1e-5 * speed).powi(2);
    let mut nearby = Vec::new();
    let mut sweep_constraints = Vec::new();
    // Earlier bodies reserve swept motion; later bodies remain stationary until resolved.
    for i in 0..bodies.len() {
        let (body, desired) = bodies[i];
        if pinned.contains(&body.entity) {
            yielding.remove(&body.entity);
            commands.entity(body.entity).remove::<TrafficYield>();
            motion.entry(body.entity).or_default().velocity = Vec2::ZERO;
            continue;
        }
        nearby.clear();
        for run in lookup.neighbourhood_runs(lookup.bucket_for_position(body.start), 1) {
            nearby.extend(run.iter().copied().filter(|j| *j != i));
        }
        let prepare_sweep = |bodies: &[(TrafficBody, Vec2)],
                             progress: &std::collections::HashMap<Entity, TravelProgress>,
                             recovering: bool,
                             constraints: &mut Vec<SweepConstraint>| {
            constraints.clear();
            for &j in &nearby {
                let other = bodies[j].0;
                let distance_squared = body.start.distance_squared(other.start);
                if distance_squared > sweep_reach_squared {
                    continue;
                }
                let passing_friend = (recovering
                    || progress
                        .get(&other.entity)
                        .is_some_and(|state| state.recovering))
                    && friendly(body.entity, other.entity)
                    && !holds_engagement(body.entity)
                    && !holds_engagement(other.entity);
                // Invalid initial overlaps may separate, but cannot deepen.
                let required = if passing_friend {
                    0.0
                } else {
                    diameter.min(distance_squared.sqrt())
                };
                constraints.push(SweepConstraint::new(body.start, other, required));
            }
        };
        // Successive candidates usually conflict with the same body, so it is checked first.
        let last_conflict = std::cell::Cell::new(0);
        let safe_motion = |constraints: &[SweepConstraint], delta: Vec2| {
            let separated = |constraint: &SweepConstraint| constraint.clears(body.start, delta);
            let first = last_conflict.get();
            if constraints
                .get(first)
                .is_some_and(|constraint| !separated(constraint))
            {
                return false;
            }
            if let Some(conflict) = constraints
                .iter()
                .enumerate()
                .position(|(j, constraint)| j != first && !separated(constraint))
            {
                last_conflict.set(conflict);
                return false;
            }
            physical.movement_clear(body.start, body.start + delta)
        };
        if is_engaged(body.entity) {
            let speed = desired.length();
            prepare_sweep(&bodies, &progress, false, &mut sweep_constraints);
            let applied = continuous_velocity(Vec2::ZERO, desired, speed, speed, |delta| {
                safe_motion(&sweep_constraints, delta)
            });
            motion.get_mut(&body.entity).unwrap().velocity = applied;
            bodies[i].0.delta = applied;
            continue;
        }
        if let Some(state) = progress.get_mut(&body.entity) {
            // A returning yield follows its recorded trail, which can be obstructed even when the original route direction is clear or pending.
            let recovery_intent = yielding
                .get(&body.entity)
                .filter(|state| state.returning)
                .and_then(|state| state.trail.last())
                .map_or(desired, |point| {
                    clamp_velocity(*point - body.start, tuning.logistics.bot_speed)
                });
            let yielding_to = yielding
                .get(&body.entity)
                .filter(|state| !state.returning)
                .map(|state| state.to);
            let obstructed_by_friend = remaining_travel
                .get(body.entity)
                .is_ok_and(|remaining| remaining.0 > 0.01)
                && nearby.iter().any(|j| {
                    let (other, wanted) = bodies[*j];
                    if !friendly(body.entity, other.entity) {
                        return false;
                    }
                    if swept_separation(body.start, recovery_intent, other) < diameter {
                        return true;
                    }
                    // Mutual yields can stop outside either body's individual sweep.
                    yielding_to == Some(other.entity)
                        && yielding
                            .get(&other.entity)
                            .is_some_and(|state| !state.returning && state.to == body.entity)
                        && swept_separation(
                            body.start,
                            desired,
                            TrafficBody {
                                delta: wanted,
                                ..other
                            },
                        ) < diameter
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
        let can_resume_route = yielding.contains_key(&body.entity)
            && desired.length_squared() > 0.01
            && physical.movement_clear(body.start, body.start + desired)
            && nearby.iter().all(|j| {
                let (other, wanted) = bodies[*j];
                swept_separation(
                    body.start,
                    desired,
                    TrafficBody {
                        delta: wanted,
                        ..other
                    },
                ) >= diameter
            });
        if can_resume_route {
            yielding.remove(&body.entity);
            commands.entity(body.entity).remove::<TrafficYield>();
        }
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
            && move_order(body.entity).is_some()
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
                    .then_with(|| body.key.cmp(&other.key))
                    .then_with(|| body.entity.to_bits().cmp(&other.entity.to_bits()))
                    .is_gt();
                other_precedes
                    && move_order(other.entity).is_some()
                    && toward.length() >= diameter - 0.01
                    && toward.length() < diameter + tuning.logistics.bot_speed * 4.0
                    && desired.dot(wanted) < 0.0
                    && toward.dot(desired) > 0.0
                    && swept_separation(
                        body.start,
                        desired,
                        TrafficBody {
                            delta: wanted,
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
                    && offset.length() < diameter + tuning.logistics.bot_speed * 4.0
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
                .and_then(|(_, _, _, _, marker, ..)| marker.and_then(|marker| marker.rejoin))
                .is_some_and(|point| navigation.segment_clear(body.start, point))
        {
            yielding.remove(&body.entity);
            commands.entity(body.entity).remove::<TrafficYield>();
        }
        let requested = if let Some(state) = yielding.get_mut(&body.entity) {
            while state.returning
                && state
                    .trail
                    .last()
                    .is_some_and(|point| point.distance(body.start) < 0.01)
            {
                state.trail.pop();
            }
            if state.returning {
                state.trail.last().map_or(Vec2::ZERO, |point| {
                    clamp_velocity(*point - body.start, speed)
                })
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
        } else {
            desired
        };
        let state = motion.get_mut(&body.entity).unwrap();
        let acceleration = speed * (time.delta_secs() / 0.12).clamp(0.0, 1.0);
        let local_remaining = state
            .departure
            .map(|point| point.distance(body.start))
            .or_else(|| {
                yielding
                    .get(&body.entity)
                    .filter(|state| state.returning)
                    .and_then(|state| state.trail.last().map(|point| point.distance(body.start)))
            });
        let remaining = local_remaining.or_else(|| {
            remaining_travel
                .get(body.entity)
                .ok()
                .map(|travel| travel.0)
        });
        let braking = local_remaining.is_some() || !yielding.contains_key(&body.entity);
        let limit = if braking {
            remaining.map_or(speed, |distance| speed.min(distance.max(0.0)))
        } else {
            speed
        };
        let target = if braking {
            remaining
                .filter(|distance| *distance >= WAYPOINT_REACHED_DISTANCE)
                .map_or(requested, |distance| {
                    clamp_velocity(
                        requested,
                        (acceleration * acceleration + 2.0 * acceleration * distance.max(0.0))
                            .sqrt()
                            - acceleration,
                    )
                })
        } else {
            requested
        };
        prepare_sweep(&bodies, &progress, recovering, &mut sweep_constraints);
        let mut applied =
            continuous_velocity(state.velocity, target, acceleration, limit, |delta| {
                safe_motion(&sweep_constraints, delta)
            });
        if recovering
            && queued.contains(body.entity)
            && applied != Vec2::ZERO
            && (state.separation_remainder != Vec2::ZERO || body.start + applied == body.start)
        {
            // Preserve shallow separation until it can move the world-space position.
            let intended = applied + state.separation_remainder;
            let step = (body.start + intended) - body.start;
            if step.length_squared() <= limit * limit && safe_motion(&sweep_constraints, step) {
                applied = step;
                state.separation_remainder = intended - step;
            } else {
                state.separation_remainder = Vec2::ZERO;
            }
        } else {
            state.separation_remainder = Vec2::ZERO;
        }
        state.velocity = applied;
        if let Some(state) = yielding.get_mut(&body.entity) {
            if !state.returning && applied.length_squared() > 0.0 {
                state.trail.push(body.start + applied);
                // A committed side-step or retreat clears a passage even while route distance grows.
                if applied.dot(state.side.unwrap_or(state.retreat)) > 2.0
                    && let Some(travel) = progress.get_mut(&body.entity)
                {
                    travel.last_progress = now;
                }
            }
            // A sidestep can hide the next waypoint behind a wall. If traffic also
            // prevents retreat, release the yield so navigation can replan from here.
            let stranded = desired == Vec2::ZERO
                && applied == Vec2::ZERO
                && progress
                    .get(&body.entity)
                    .is_some_and(|travel| now - travel.last_progress >= 1.0);
            if stranded || (state.returning && state.trail.is_empty()) {
                yielding.remove(&body.entity);
                commands.entity(body.entity).remove::<TrafficYield>();
            }
        }
        bodies[i].0.delta = applied;
    }
    for (entity, mut velocity, mut transform, bot, _, _, lease, _) in &mut query {
        let start = transform.translation.truncate();
        let applied_velocity = if bot.is_some() {
            bodies[indices[&entity]].0.delta
        } else {
            let proposed = clamp_velocity(velocity.value, tuning.logistics.bot_speed);
            if physical.movement_clear(start, start + proposed) {
                proposed
            } else {
                Vec2::ZERO
            }
        };
        transform.translation += applied_velocity.extend(0.);
        if let Some(mut lease) = lease {
            lease.finish_travel_step(start, transform.translation.truncate());
        }
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

    /// The full scan that velocity selection must match: the first safe candidate with the lowest score.
    fn full_scan_velocity(
        previous: Vec2,
        requested: Vec2,
        acceleration: f32,
        speed_limit: f32,
        safe: impl Fn(Vec2) -> bool,
    ) -> Vec2 {
        if requested.length_squared() == 0.0 || acceleration <= 0.0 {
            return Vec2::ZERO;
        }
        let preferred = clamp_velocity(
            previous + clamp_velocity(requested - previous, acceleration),
            speed_limit,
        );
        if safe(preferred) {
            return preferred;
        }
        let requested_speed = requested.length();
        let requested_direction = requested / requested_speed;
        let mut best = None;
        let mut best_score = f32::INFINITY;
        for ring in 0..=4 {
            for heading in 0..24 {
                let angle = heading as f32 * std::f32::consts::TAU / 24.0;
                let candidate = previous
                    + Vec2::new(angle.cos(), angle.sin()) * acceleration * ring as f32 / 4.0;
                if candidate.length_squared() > speed_limit * speed_limit + 1e-6 {
                    continue;
                }
                let lateral = candidate.perp_dot(requested_direction);
                let lost_speed = requested_speed - candidate.length();
                let score = candidate.distance_squared(requested)
                    + lateral * lateral
                    + 2.0 * lost_speed * lost_speed
                    + 0.5 * candidate.distance_squared(previous);
                if score < best_score && safe(candidate) {
                    best = Some(candidate);
                    best_score = score;
                }
            }
        }
        best.unwrap_or(Vec2::ZERO)
    }

    #[test]
    fn sweep_clearance_matches_the_swept_separation_test() {
        use rand::{RngExt, SeedableRng, rngs::StdRng};

        let mut rng = StdRng::seed_from_u64(0x5EED_5EE9);
        let (mut rejected, mut ended_inside) = (0, 0);
        for case in 0..200_000 {
            let scale = [1.0, 10.0, 100.0, 1_000.0][case % 4];
            let mut vector = |scale: f32| {
                Vec2::new(
                    rng.random_range(-scale..scale),
                    rng.random_range(-scale..scale),
                )
            };
            let start = vector(1_000.0);
            let mut delta = vector(scale * 0.1);
            let other = TrafficBody {
                entity: entity(1),
                start: start + vector(scale),
                delta: if case % 3 == 0 {
                    Vec2::ZERO
                } else {
                    vector(scale * 0.1)
                },
                key: 1,
            };
            // Matching motion leaves almost no relative travel.
            if case % 7 == 0 {
                delta = other.delta + Vec2::splat(1e-5);
            }
            let required = match case % 5 {
                0 => 0.0,
                1 => BODY_DIAMETER,
                _ => BODY_DIAMETER.min(start.distance(other.start)),
            };
            let expected = swept_separation(start, delta, other) + 0.001 >= required;
            let constraint = SweepConstraint::new(start, other, required);
            // Ruling a motion out early must never contradict the full test.
            if constraint.ends_inside(start, delta) {
                assert!(!expected, "case {case} ends inside yet clears");
                ended_inside += 1;
            }
            assert_eq!(
                constraint.clears(start, delta),
                expected,
                "case {case}: start {start:?}, delta {delta:?}, other {:?} moving {:?}",
                other.start,
                other.delta,
            );
            rejected += usize::from(!expected);
        }
        assert!(rejected > 10_000, "the cases must include blocked sweeps");
        assert!(
            ended_inside > 1_000,
            "the cases must include motions ending inside"
        );
    }

    #[test]
    fn velocity_choice_matches_a_full_scan_of_candidates() {
        use rand::{RngExt, SeedableRng, rngs::StdRng};

        let mut rng = StdRng::seed_from_u64(0x5EED_0089);
        let vector = |rng: &mut StdRng, scale: f32| {
            Vec2::new(
                rng.random_range(-scale..scale),
                rng.random_range(-scale..scale),
            )
        };
        let mut stopped = 0;
        for case in 0..20_000 {
            // Axis-aligned requests from rest score mirrored candidates equally.
            let axis_aligned = case % 4 == 1;
            let previous = if axis_aligned || case % 3 == 0 {
                Vec2::ZERO
            } else {
                vector(&mut rng, 5.0)
            };
            let requested = if axis_aligned {
                [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y][rng.random_range(0..4)]
                    * rng.random_range(1..6) as f32
            } else {
                vector(&mut rng, 6.0)
            };
            let acceleration = if case % 2 == 0 {
                5.0
            } else {
                rng.random_range(0.1..5.0)
            };
            let speed_limit = rng.random_range(0.0..6.0);
            // Bodies around the mover block the motions that come too close to them.
            let blockers: Vec<_> = (0..rng.random_range(0..6))
                .map(|_| (vector(&mut rng, 8.0), rng.random_range(0.5..6.0)))
                .collect();
            let safe = |delta: Vec2| {
                blockers
                    .iter()
                    .all(|&(center, radius)| delta.distance(center) >= radius)
            };
            let expected = full_scan_velocity(previous, requested, acceleration, speed_limit, safe);
            let actual = continuous_velocity(previous, requested, acceleration, speed_limit, safe);
            assert_eq!(
                actual.to_array().map(f32::to_bits),
                expected.to_array().map(f32::to_bits),
                "case {case}: previous {previous:?}, requested {requested:?}",
            );
            stopped += usize::from(expected == Vec2::ZERO);
        }
        assert!(stopped > 100, "the cases must include fully blocked motion");
    }

    #[test]
    fn separation_scales_with_overlap_and_only_pushes_nearby_pairs() {
        let first = entity(1);
        let second = entity(2);
        let distant = entity(3);
        for (distance, expected_force) in [(34.0, BOT_SEPARATION_FORCE), (51.0, 0.75)] {
            let deltas = separation_deltas(&[
                SeparationEntry {
                    entity: first,
                    position: Vec2::ZERO,
                    key: first.to_bits(),
                },
                SeparationEntry {
                    entity: second,
                    position: Vec2::X * distance,
                    key: second.to_bits(),
                },
                SeparationEntry {
                    entity: distant,
                    position: Vec2::splat(crate::nanobot::consts::BOT_RADIUS * 10.0),
                    key: distant.to_bits(),
                },
            ]);

            let first_delta = deltas.iter().find(|(id, _)| *id == first).unwrap().1;
            let second_delta = deltas.iter().find(|(id, _)| *id == second).unwrap().1;
            let distant_delta = deltas.iter().find(|(id, _)| *id == distant).unwrap().1;
            assert_abs_diff_eq!(first_delta.x, -expected_force, epsilon = 1e-5);
            assert_abs_diff_eq!(first_delta.y, 0.0, epsilon = 1e-5);
            assert_abs_diff_eq!(second_delta.x, expected_force, epsilon = 1e-5);
            assert_abs_diff_eq!(second_delta.y, 0.0, epsilon = 1e-5);
            assert_abs_diff_eq!(distant_delta.x, 0.0, epsilon = 1e-5);
            assert_abs_diff_eq!(distant_delta.y, 0.0, epsilon = 1e-5);
        }
    }

    #[test]
    fn coincident_pair_separation_is_deterministic_and_finite() {
        let entries = [
            SeparationEntry {
                entity: entity(1),
                position: Vec2::ZERO,
                key: 1,
            },
            SeparationEntry {
                entity: entity(2),
                position: Vec2::ZERO,
                key: 2,
            },
        ];
        let first = separation_deltas(&entries);
        let second = separation_deltas(&entries);

        assert_eq!(first, second);
        assert!(first.iter().all(|(_, delta)| delta.is_finite()));
        assert!((first[0].1 + first[1].1).length_squared() < 1e-6);
    }
}
