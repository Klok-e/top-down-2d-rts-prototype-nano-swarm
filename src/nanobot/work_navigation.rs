use super::move_system::{ActiveWork, AssignedWork, TransitBody, TransitMotion};
use super::route_following::WAYPOINT_REACHED_DISTANCE;
use super::{
    DefenderEngagements, DirectMovementComponent, ProgressChecker, STOP_THRESHOLD, TrafficYield,
    VelocityComponent,
};
use crate::spatial::{FixedSpatialBuckets, PackedSpatialBuckets};
use crate::tuning::Tuning;
use bevy::prelude::*;

/// A loaded bot waiting for a separate position; yielding does not reset queue age.
#[derive(Component, Debug)]
pub struct WaitingForWork;

/// The observable stage of a bot approaching an exterior work goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproachPhase {
    Travelling,
    Searching,
    Waiting,
}

/// Local searching and yielding preserve the age of the original work approach.
#[derive(Component, Debug, Clone, Copy)]
pub struct WorkApproach {
    pub region: super::InteractionRegion,
    pub phase: ApproachPhase,
    pub since: f64,
}

/// Distance along the remaining static route for movement braking.
#[derive(Component, Debug, Clone, Copy)]
pub struct RemainingTravel(pub f32);

/// The current standing position is reserved for work while a task needs it.
#[derive(Component)]
pub struct AdmittedToWork;

/// Work effects are suspended until the bot can occupy an exclusive Work Position.
#[derive(Component, Debug)]
pub struct WorkBlocked;

/// A saturated work perimeter is retried after allowing other work to be selected.
#[derive(Component)]
pub struct RejectedWorkGoal {
    pub region: super::InteractionRegion,
    remaining_seconds: f32,
}

#[allow(clippy::type_complexity)]
pub fn work_standing_system(
    mut commands: Commands,
    bots: Query<
        (
            Entity,
            &Transform,
            TransitBody,
            Has<AdmittedToWork>,
            Has<WorkBlocked>,
            Option<&super::Commitment>,
        ),
        With<super::Nanobot>,
    >,
    active_work: Query<(), ActiveWork>,
    assigned_work: Query<(), AssignedWork>,
    engagements: Option<Res<DefenderEngagements>>,
    mut rejected: Query<(Entity, &mut RejectedWorkGoal)>,
    time: Res<Time<Fixed>>,
) {
    for (entity, mut rejected) in &mut rejected {
        rejected.remaining_seconds -= time.delta_secs();
        if rejected.remaining_seconds <= 0.0 {
            commands.entity(entity).remove::<RejectedWorkGoal>();
        }
    }
    let occupants: Vec<_> = bots
        .iter()
        .map(|(entity, transform, body, admitted, _, commitment)| {
            let motion = body.motion(entity, engagements.as_deref());
            let has_work = active_work.contains(entity)
                || assigned_work.contains(entity)
                || commitment == Some(&super::Commitment::Working);
            let standing = !body.has_movement_order() && !motion.combat_controlled;
            let priority = if standing && has_work {
                if admitted {
                    StandingPriority::Admitted
                } else {
                    StandingPriority::Assigned
                }
            } else {
                StandingPriority::Unassigned
            };
            Occupant {
                entity,
                position: transform.translation.truncate(),
                motion,
                priority,
            }
        })
        .collect();
    let occupancy = Occupancy::new(occupants.iter().copied());
    for occupant in occupants {
        let (_, _, body, admitted, was_blocked, _) = bots.get(occupant.entity).unwrap();
        if admitted && occupant.priority != StandingPriority::Admitted {
            commands.entity(occupant.entity).remove::<AdmittedToWork>();
        }
        // An existing work effect can retain its assignment while moving to a new standing point.
        let moving_work =
            active_work.contains(occupant.entity) && body.moving_in_work_region(occupant.position);
        let blocked = moving_work
            || !occupancy.clear_at(occupant.entity, occupant.position, |other| {
                other.motion.can_pass(occupant.motion)
                    || (occupant.priority != StandingPriority::Unassigned
                        && occupant.motion.friendly_transit(other.motion)
                        && (occupant.priority, occupant.entity.to_bits())
                            < (other.priority, other.entity.to_bits()))
            });
        if blocked && !was_blocked {
            commands.entity(occupant.entity).insert(WorkBlocked);
        } else if !blocked && was_blocked {
            commands.entity(occupant.entity).remove::<WorkBlocked>();
        }
    }
}

#[derive(Default)]
pub struct WorkApproaches {
    entries: std::collections::HashMap<Entity, ApproachEntry>,
    /// Start-independent work candidates by region.
    candidate_parts: bevy::platform::collections::HashMap<
        super::interaction::InteractionBits,
        super::interaction::CandidateParts,
    >,
}

struct ApproachEntry {
    region: super::InteractionRegion,
    since: f64,
    waiting_since: Option<f64>,
    selected: Option<Vec2>,
    connection_origin: Option<Vec2>,
    nearby: bool,
    rejected: Vec<Vec2>,
    revision: u64,
}

pub struct ActiveRoute {
    destination: Vec2,
    requested_start: Vec2,
    task: Option<super::InteractionRegion>,
    stop_radius: f32,
    interaction: Option<super::InteractionRegion>,
    waypoints: Vec<Vec2>,
    current: usize,
    follower: super::route_following::RouteFollower,
    revision: u64,
    pending: Option<crate::navigation::RouteRequestId>,
}

impl ActiveRoute {
    /// Preserve the reachable prefix so a replacement search does not freeze safe travel.
    fn trim_at_obstruction(
        &mut self,
        position: Vec2,
        navigation: &crate::navigation::Navigation,
    ) -> bool {
        let next = self.follower.target_index(self.current);
        let mut previous = position;
        for index in next..self.waypoints.len() {
            let point = self.waypoints[index];
            if !navigation.movement_clear(previous, point) {
                let mut clear = 0.0;
                let mut blocked = 1.0;
                for _ in 0..12 {
                    let fraction = (clear + blocked) * 0.5;
                    if navigation.movement_clear(previous, previous.lerp(point, fraction)) {
                        clear = fraction;
                    } else {
                        blocked = fraction;
                    }
                }
                let end = previous.lerp(point, clear);
                self.waypoints.truncate(index);
                if end.distance(previous) > STOP_THRESHOLD {
                    self.waypoints.push(end);
                }
                return true;
            }
            previous = point;
        }
        false
    }
}

/// Regions whose start-independent work candidates stay cached; geometry is
/// static, so the cache only bounds memory.
const MAX_CACHED_CANDIDATE_REGIONS: usize = 4096;

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn move_velocity_system(
    mut commands: Commands,
    mut approaches: Local<WorkApproaches>,
    mut routes: Local<std::collections::HashMap<Entity, ActiveRoute>>,
    generation: Option<Res<crate::session_lifecycle::SessionGeneration>>,
    mut previous_generation: Local<Option<u64>>,
    mut previous_corridor_multiplier: Local<Option<f32>>,
    mut bots: Query<(
        Entity,
        &DirectMovementComponent,
        &Transform,
        &mut VelocityComponent,
        Option<&super::NanobotType>,
        Option<&super::SwarmMember>,
        Option<&super::ClearingEvacuation>,
        Option<&TrafficYield>,
        Option<&mut super::RegionalLease>,
    )>,
    bodies: Query<(Entity, &Transform, TransitBody), With<super::Nanobot>>,
    waiting: Query<&DirectMovementComponent, With<WaitingForWork>>,
    loads: Query<&super::Cargo>,
    tuning: Res<Tuning>,
    navigation: Res<crate::navigation::Navigation>,
    time: Res<Time<Fixed>>,
    engagements: Option<Res<DefenderEngagements>>,
) {
    use crate::navigation::{ConnectivityStatus, RouteGoal, RoutePriority, RouteStatus};
    let is_engaged = |entity| {
        engagements
            .as_ref()
            .is_some_and(|value| value.controls_movement(entity))
    };
    if let Some(generation) = generation.as_deref().map(|generation| generation.0)
        && *previous_generation != Some(generation)
    {
        approaches.entries.clear();
        routes.clear();
        *previous_generation = Some(generation);
    }
    if *previous_corridor_multiplier != Some(tuning.logistics.corridor_cost_multiplier) {
        for route in routes.values() {
            if let Some(id) = route.pending {
                navigation.cancel(id);
            }
        }
        routes.clear();
        *previous_corridor_multiplier = Some(tuning.logistics.corridor_cost_multiplier);
    }
    routes.retain(|entity, route| {
        if bots.contains(*entity) && !is_engaged(*entity) {
            true
        } else {
            if let Some(id) = route.pending {
                navigation.cancel(id);
            }
            if bodies.contains(*entity) {
                commands.entity(*entity).remove::<RemainingTravel>();
            }
            false
        }
    });
    let now = time.elapsed_secs_f64();
    approaches.entries.retain(|entity, _| {
        let active = bots.contains(*entity) && !is_engaged(*entity);
        if !active && bodies.contains(*entity) {
            commands
                .entity(*entity)
                .remove::<(RemainingTravel, WaitingForWork, WorkApproach)>();
        }
        active
    });

    for (entity, destination, ..) in &bots {
        if is_engaged(entity) {
            continue;
        }
        if let Some(region) = destination.interaction {
            let entry = approaches.entries.entry(entity).or_insert(ApproachEntry {
                region,
                since: now,
                waiting_since: None,
                selected: None,
                connection_origin: None,
                nearby: false,
                rejected: Vec::new(),
                revision: navigation.revision(),
            });
            if entry.region != region {
                *entry = ApproachEntry {
                    region,
                    since: now,
                    waiting_since: None,
                    selected: None,
                    connection_origin: None,
                    nearby: false,
                    rejected: Vec::new(),
                    revision: navigation.revision(),
                };
            }
        } else {
            approaches.entries.remove(&entity);
            commands.entity(entity).remove::<WorkApproach>();
        }
    }
    let mut ordered: Vec<_> = bots
        .iter()
        .filter_map(|(entity, ..)| (!is_engaged(entity)).then_some(entity))
        .collect();
    ordered.sort_by_key(|entity| {
        let approach = approaches.entries.get(entity);
        (
            approach
                .and_then(|entry| entry.waiting_since)
                .map_or(u64::MAX, f64::to_bits),
            approach.map_or(u64::MAX, |entry| entry.since.to_bits()),
            entity.to_bits(),
        )
    });
    let occupancy = Occupancy::new(bodies.iter().map(|(entity, transform, body)| Occupant {
        entity,
        position: transform.translation.truncate(),
        motion: body.motion(entity, engagements.as_deref()),
        priority: StandingPriority::Unassigned,
    }));
    let mut claims = FixedSpatialBuckets::new(2.0 * crate::navigation::BODY_RADIUS);
    for entity in ordered {
        let Ok((
            entity,
            destination,
            transform,
            mut velocity,
            kind,
            member,
            evacuation,
            yielding,
            lease,
        )) = bots.get_mut(entity)
        else {
            continue;
        };
        commands.entity(entity).remove::<RemainingTravel>();
        let keep_order_after_evacuation = evacuation.is_some_and(|evacuation| {
            destination.interaction.is_some() || destination.xy != evacuation.goal
        });
        let evacuation_destination = evacuation.map(|evacuation| DirectMovementComponent {
            xy: evacuation.goal,
            stop_radius: 0.0,
            interaction: None,
            speed: None,
        });
        let original = evacuation_destination.as_ref().unwrap_or(destination);
        let mut selected_destination = DirectMovementComponent {
            xy: original.xy,
            stop_radius: original.stop_radius,
            interaction: original.interaction,
            speed: original.speed,
        };
        let position = transform.translation.truncate();
        let mut work_ready = true;
        let mut connection_origin = None;
        if let Some(region) = selected_destination.interaction {
            let entry = approaches.entries.get_mut(&entity).unwrap();
            if entry.revision != navigation.revision() {
                entry.rejected.clear();
                entry.connection_origin = None;
                entry.revision = navigation.revision();
            }
            let distance = position.distance(region.approach(position));
            let threshold = if entry.nearby { 3.0 } else { 2.0 } * crate::navigation::CELL_WIDTH;
            entry.nearby = distance <= threshold;
            let nearby = entry.nearby;
            let mut phase = if nearby {
                ApproachPhase::Searching
            } else {
                ApproachPhase::Travelling
            };
            commands.entity(entity).insert(WorkApproach {
                region,
                phase,
                since: entry.since,
            });
            let requester = bodies.get(entity).map_or(
                TransitMotion {
                    swarm: member.map_or(super::SwarmId::PLAYER, |member| member.0),
                    travelling: true,
                    combat_controlled: false,
                },
                |(_, _, body)| body.motion(entity, engagements.as_deref()),
            );
            let open_standing_position = |point| {
                occupancy.clear_at(entity, point, |other| {
                    other.motion.can_pass(requester)
                        || (requester.friendly_transit(other.motion)
                            && waiting
                                .get(other.entity)
                                .is_ok_and(|order| order.interaction == Some(region)))
                }) && space_free(entity, point, &claims)
            };
            let free = |point: Vec2| {
                navigation.point_clear(point) && (!nearby || open_standing_position(point))
            };
            if approaches.candidate_parts.len() > MAX_CACHED_CANDIDATE_REGIONS {
                approaches.candidate_parts.clear();
            }
            let parts = approaches
                .candidate_parts
                .entry(region.bits())
                .or_insert_with(|| region.candidate_parts());
            let mut candidates = region.work_candidates_from(parts, position);
            let entry = approaches.entries.get(&entity).unwrap();
            if let Some(old) = entry.selected {
                candidates.insert(0, old);
            }
            // A clear position already in reach needs no further travel.
            if region.contains(position) {
                candidates.insert(0, position);
            }
            // Usable candidates are evaluated lazily: selection reads only the
            // retained point, the first clear one and the first one.
            let usable = |point: &Vec2| free(*point) && !entry.rejected.contains(point);
            let mut usable_candidates = candidates.iter().copied().filter(usable).peekable();
            let first_usable = usable_candidates.peek().copied();
            let mut connected = Vec::new();
            if first_usable.is_none() && region.deposit_tile_band().is_some() {
                let (status, origin) = navigation
                    .query_connectivity_with_origin(position, RouteGoal::Interaction(region));
                match status {
                    ConnectivityStatus::Connected { endpoint } if free(endpoint) => {
                        let can_reattach = !navigation.movement_clear(position, endpoint)
                            && navigation.movement_clear(position, origin)
                            && navigation.movement_clear(origin, endpoint);
                        if can_reattach || !entry.rejected.contains(&endpoint) {
                            connected.push(endpoint);
                        }
                        if can_reattach {
                            connection_origin = Some(origin);
                        }
                    }
                    ConnectivityStatus::Pending => continue,
                    _ => {}
                }
            }
            // Retain a usable commitment, otherwise prefer an immediately clear approach.
            let retained = entry
                .selected
                .filter(|point| {
                    if first_usable.is_some() {
                        candidates.contains(point) && usable(point)
                    } else {
                        connected.contains(point)
                    }
                })
                .filter(|point| {
                    navigation.movement_clear(position, *point)
                        || routes.get(&entity).is_some_and(|route| {
                            route.destination == *point
                                && (route.pending.is_some() || !route.waypoints.is_empty())
                        })
                });
            let selected = retained.or_else(|| {
                let clear = |point: &Vec2| navigation.movement_clear(position, *point);
                if first_usable.is_some() {
                    usable_candidates.find(clear).or(first_usable)
                } else {
                    connected
                        .iter()
                        .copied()
                        .find(clear)
                        .or(connected.first().copied())
                }
            });
            if let Some(point) = selected {
                work_ready = nearby
                    && region.contains(position)
                    && open_standing_position(position)
                    && position.distance(point) <= STOP_THRESHOLD;
                if nearby {
                    let claimed_position = if work_ready { position } else { point };
                    claims.insert(claimed_position, (entity, claimed_position));
                }
                let entry = approaches.entries.get_mut(&entity).unwrap();
                if entry.selected != Some(point) {
                    entry.connection_origin = None;
                }
                entry.selected = Some(point);
                if connection_origin.is_some() {
                    entry.connection_origin = connection_origin;
                }
                // Route to the selected perimeter position, never the occupied nearest face.
                selected_destination.xy = point;
                selected_destination.interaction = None;
                commands.entity(entity).remove::<WaitingForWork>();
            } else {
                let carrying = loads.get(entity).is_ok_and(|cargo| cargo.amount > 0);
                if carrying || kind == Some(&super::NanobotType::Defender) {
                    let entry = approaches.entries.get_mut(&entity).unwrap();
                    if carrying && kind == Some(&super::NanobotType::Hauler) {
                        entry.waiting_since.get_or_insert(now);
                    }
                    phase = ApproachPhase::Waiting;
                    commands.entity(entity).insert((
                        WaitingForWork,
                        WorkApproach {
                            region,
                            phase,
                            since: entry.since,
                        },
                    ));
                } else {
                    commands
                        .entity(entity)
                        .insert((
                            super::Commitment::Idle,
                            RejectedWorkGoal {
                                region,
                                remaining_seconds: 1.0,
                            },
                        ))
                        .remove::<(
                            super::BuildAssignment,
                            super::BuildProgress,
                            super::ReturningToStockpile,
                        )>()
                        .remove::<(
                            DirectMovementComponent,
                            super::RegionalLease,
                            super::GatherAssignment,
                            super::ExtractProgress,
                            super::PlannedStructureClaim,
                            super::PlannedStructureProgress,
                            super::MaintenanceAssignment,
                            super::MaintenanceProgress,
                            super::HaulerAssignment,
                            super::HaulerLoading,
                            super::LogisticsReservation,
                            super::Cargo,
                            WaitingForWork,
                            WorkApproach,
                        )>();
                }
                continue;
            }
        } else {
            commands
                .entity(entity)
                .remove::<(WaitingForWork, WorkApproach)>();
        }
        let destination = &selected_destination;
        let position = transform.translation.truncate();
        let stop = destination.stop_radius.max(STOP_THRESHOLD);
        if work_ready
            && destination
                .interaction
                .map_or(position.distance(destination.xy) <= stop, |region| {
                    region.contains(position)
                })
            && navigation.point_clear(position)
        {
            commands
                .entity(entity)
                .remove::<(ProgressChecker, WorkApproach, WaitingForWork)>();
            if original.interaction.is_some() {
                commands.entity(entity).insert(AdmittedToWork);
            }
            if !keep_order_after_evacuation {
                commands.entity(entity).remove::<DirectMovementComponent>();
            }
            if let Some(route) = routes.remove(&entity)
                && let Some(id) = route.pending
            {
                navigation.cancel(id);
            }
            continue;
        }
        // Take the last short step when the current position is outside the work tile or occupied.
        let last_step = destination.xy - position;
        if last_step.length_squared() < WAYPOINT_REACHED_DISTANCE.powi(2)
            && original
                .interaction
                .is_some_and(|region| region.contains(destination.xy))
            && navigation.movement_clear(position, destination.xy)
        {
            commands
                .entity(entity)
                .insert(RemainingTravel(last_step.length()));
            velocity.value += last_step;
            continue;
        }
        if original.interaction.is_some()
            && let Some(entry) = approaches.entries.get_mut(&entity)
            && let Some(origin) = entry.connection_origin
        {
            if navigation.movement_clear(position, origin)
                && navigation.movement_clear(origin, destination.xy)
            {
                // Stop precisely at the start of the proven leg before turning into the work site.
                let step = origin - position;
                if step == Vec2::ZERO {
                    entry.connection_origin = None;
                } else if let Some(mut lease) = lease {
                    lease.begin_travel_step(position, origin);
                }
                if let Some(route) = routes.remove(&entity)
                    && let Some(id) = route.pending
                {
                    navigation.cancel(id);
                }
                commands
                    .entity(entity)
                    .insert(RemainingTravel(step.length()));
                velocity.value += super::clamp_velocity(
                    step,
                    destination.speed.unwrap_or(tuning.logistics.bot_speed),
                );
                continue;
            }
            entry.connection_origin = None;
        }
        let hauler = kind == Some(&super::NanobotType::Hauler);
        let direct = navigation.movement_clear(position, destination.xy)
            && (!hauler
                || original.interaction.is_some_and(|region| {
                    position.distance(region.approach(position))
                        <= 2.0 * crate::navigation::CELL_WIDTH
                }));
        if direct
            && routes.get(&entity).is_none_or(|route| {
                route.pending.is_some()
                    || route.task != original.interaction
                    || route.current != 0
                    || route.waypoints.as_slice() != [destination.xy]
            })
        {
            if let Some(old) = routes.remove(&entity)
                && let Some(id) = old.pending
            {
                navigation.cancel(id);
            }
            routes.insert(
                entity,
                ActiveRoute {
                    destination: destination.xy,
                    requested_start: position,
                    task: original.interaction,
                    stop_radius: stop,
                    interaction: destination.interaction,
                    waypoints: vec![destination.xy],
                    current: 0,
                    follower: Default::default(),
                    revision: navigation.revision(),
                    pending: None,
                },
            );
        }
        let invalidated = routes.get_mut(&entity).is_some_and(|route| {
            if route.revision == navigation.revision() {
                return false;
            }
            route.revision = navigation.revision();
            route.waypoints.is_empty() || route.trim_at_obstruction(position, &navigation)
        });
        let needs_route = invalidated
            || routes.get(&entity).is_none_or(|route| {
                (route.pending.is_none()
                    && route.destination != destination.xy
                    && (original.interaction.is_some()
                        || route.destination.distance(destination.xy)
                            > crate::navigation::CELL_WIDTH
                        || route.current >= route.waypoints.len()))
                    || (route.pending.is_none()
                        && !route.waypoints.is_empty()
                        && route.current >= route.waypoints.len()
                        && position.distance(destination.xy) > STOP_THRESHOLD)
                    || route.task != original.interaction
                    || route.stop_radius != stop
                    || route.interaction != destination.interaction
                    || (route.waypoints.is_empty() && route.revision != navigation.revision())
                    || route.pending.is_none()
                        && yielding.is_none()
                        && route
                            .waypoints
                            .get(route.follower.target_index(route.current))
                            .is_some_and(|next| !navigation.movement_clear(position, *next))
            });
        if needs_route {
            let swarm = member.map_or(super::SwarmId::PLAYER, |member| member.0);
            let priority = if evacuation.is_some() {
                RoutePriority::Clearing
            } else if routes.contains_key(&entity) {
                RoutePriority::Invalidated
            } else {
                RoutePriority::Routine
            };
            if let Some(route) = routes.get(&entity)
                && let Some(id) = route.pending
            {
                navigation.cancel(id);
            }
            let goal = if let Some(region) = destination.interaction {
                RouteGoal::Interaction(region)
            } else if stop > STOP_THRESHOLD {
                RouteGoal::Range {
                    center: destination.xy,
                    radius: stop,
                }
            } else {
                RouteGoal::Point(destination.xy)
            };
            let pending =
                Some(navigation.request_owned(position, goal, swarm, hauler, priority, entity));
            let (waypoints, current, follower) = routes
                .get(&entity)
                .filter(|route| {
                    route.task == original.interaction
                        && route.interaction == destination.interaction
                        && route
                            .waypoints
                            .get(route.follower.target_index(route.current))
                            .is_some_and(|next| navigation.movement_clear(position, *next))
                })
                .map_or_else(
                    || {
                        (
                            Vec::new(),
                            0,
                            super::route_following::RouteFollower::default(),
                        )
                    },
                    |route| (route.waypoints.clone(), route.current, route.follower),
                );
            routes.insert(
                entity,
                ActiveRoute {
                    destination: destination.xy,
                    requested_start: position,
                    task: original.interaction,
                    stop_radius: stop,
                    waypoints,
                    interaction: destination.interaction,
                    current,
                    follower,
                    revision: navigation.revision(),
                    pending,
                },
            );
        }
        let route = routes.get_mut(&entity).expect("route was resolved");
        if let Some(id) = route.pending {
            match navigation.poll(id) {
                RouteStatus::Pending => {}
                RouteStatus::Found(found) => {
                    route.waypoints = found.waypoints;
                    route.current = super::route_following::rejoin_route(
                        position,
                        route.requested_start,
                        &mut route.waypoints,
                        &navigation,
                    );
                    route.follower = Default::default();
                    route.pending = None;
                    navigation.cancel(id);
                }
                RouteStatus::Unreachable => {
                    if let Some(entry) = approaches.entries.get_mut(&entity) {
                        entry.rejected.push(route.destination);
                        entry.selected = None;
                    }
                    route.waypoints.clear();
                    route.current = 0;
                    route.follower = Default::default();
                    route.pending = None;
                    navigation.cancel(id);
                }
            }
        }
        let speed = destination.speed.unwrap_or(tuning.logistics.bot_speed);
        let step = route.follower.route_step(
            position,
            &route.waypoints,
            &mut route.current,
            speed,
            hauler,
            &navigation,
        );
        let next_index = route.follower.target_index(route.current);
        if let Some(next) = route.waypoints.get(next_index) {
            if let Some(mut lease) = lease
                && evacuation.is_none()
                && step != Vec2::ZERO
            {
                lease.begin_travel_step(position, *next);
            }
            let remaining = position.distance(*next)
                + route.waypoints[next_index..]
                    .windows(2)
                    .map(|segment| segment[0].distance(segment[1]))
                    .sum::<f32>();
            commands.entity(entity).insert(RemainingTravel(remaining));
            if yielding.is_some() {
                commands.entity(entity).insert(TrafficYield {
                    rejoin: Some(*next),
                });
            }
            velocity.value += step;
        }
    }
}

fn space_free(
    entity: Entity,
    position: Vec2,
    occupancy: &FixedSpatialBuckets<(Entity, Vec2)>,
) -> bool {
    clear_of(
        entity,
        position,
        occupancy
            .neighbourhood(occupancy.bucket_for_position(position), 1)
            .flat_map(|(_, entries)| entries),
    )
}

/// Whether no body other than `entity` among `nearby` overlaps `position`.
fn clear_of<'a>(
    entity: Entity,
    position: Vec2,
    nearby: impl IntoIterator<Item = &'a (Entity, Vec2)>,
) -> bool {
    nearby.into_iter().all(|(other, other_position)| {
        *other == entity
            || position.distance_squared(*other_position)
                >= (2.0 * crate::navigation::BODY_RADIUS - 0.001).powi(2)
    })
}

#[derive(Clone, Copy)]
struct Occupant {
    entity: Entity,
    position: Vec2,
    motion: TransitMotion,
    priority: StandingPriority,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum StandingPriority {
    Admitted,
    Assigned,
    Unassigned,
}

struct Occupancy(PackedSpatialBuckets<Occupant>);

impl Occupancy {
    fn new(bodies: impl IntoIterator<Item = Occupant>) -> Self {
        Self(PackedSpatialBuckets::new(
            2.0 * crate::navigation::BODY_RADIUS,
            bodies.into_iter().map(|body| (body.position, body)),
        ))
    }

    fn clear_at(
        &self,
        entity: Entity,
        position: Vec2,
        can_share: impl Fn(&Occupant) -> bool,
    ) -> bool {
        self.0
            .neighbourhood_runs(self.0.bucket_for_position(position), 1)
            .flatten()
            .all(|other| {
                other.entity == entity
                    || position.distance_squared(other.position)
                        >= (2.0 * crate::navigation::BODY_RADIUS - 0.001).powi(2)
                    || can_share(other)
            })
    }
}
