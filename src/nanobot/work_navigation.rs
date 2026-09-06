use super::{
    DirectMovementComponent, ProgressChecker, STOP_THRESHOLD, TrafficYield, VelocityComponent,
};
use crate::game_settings::GameSettings;
use crate::spatial::FixedSpatialBuckets;
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

/// Distance along the remaining static route, used to recognize real travel progress.
#[derive(Component, Debug, Clone, Copy)]
pub struct RemainingTravel(pub f32);

/// Work effects are suspended while the body overlaps another body or recovers.
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
    bots: Query<(Entity, &Transform, Has<super::CongestionRecovery>), With<super::Nanobot>>,
    mut rejected: Query<(Entity, &mut RejectedWorkGoal)>,
    time: Res<Time<Fixed>>,
) {
    for (entity, mut rejected) in &mut rejected {
        rejected.remaining_seconds -= time.delta_secs();
        if rejected.remaining_seconds <= 0.0 {
            commands.entity(entity).remove::<RejectedWorkGoal>();
        }
    }
    let mut occupancy = FixedSpatialBuckets::new(2.0 * crate::navigation::BODY_RADIUS);
    for (entity, transform, _) in &bots {
        occupancy.insert(
            transform.translation.truncate(),
            (entity, transform.translation.truncate()),
        );
    }
    for (entity, transform, recovering) in &bots {
        let blocked =
            recovering || !space_free(entity, transform.translation.truncate(), &occupancy);
        if blocked {
            commands.entity(entity).insert(WorkBlocked);
        } else {
            commands.entity(entity).remove::<WorkBlocked>();
        }
    }
}

#[derive(Default)]
pub struct WorkApproaches {
    entries: std::collections::HashMap<Entity, ApproachEntry>,
}

struct ApproachEntry {
    region: super::InteractionRegion,
    since: f64,
    selected: Option<Vec2>,
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

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn move_velocity_system(
    mut commands: Commands,
    mut approaches: Local<WorkApproaches>,
    mut routes: Local<std::collections::HashMap<Entity, ActiveRoute>>,
    mut bots: Query<(
        Entity,
        &DirectMovementComponent,
        &Transform,
        &mut VelocityComponent,
        Option<&super::NanobotType>,
        Option<&super::SwarmMember>,
        Option<&super::ClearingEvacuation>,
        Option<&TrafficYield>,
    )>,
    bodies: Query<(Entity, &Transform, Has<super::CongestionRecovery>), With<super::Nanobot>>,
    loads: Query<&super::Cargo>,
    game_settings: Res<GameSettings>,
    navigation: Res<crate::navigation::Navigation>,
    time: Res<Time<Fixed>>,
) {
    use crate::navigation::{RouteGoal, RoutePriority, RouteStatus};
    routes.retain(|entity, route| {
        if bots.contains(*entity) {
            true
        } else {
            if let Some(id) = route.pending {
                navigation.cancel(id);
            }
            if bodies.contains(*entity) {
                commands
                    .entity(*entity)
                    .remove::<(RemainingTravel, WaitingForWork, WorkApproach)>();
            }
            false
        }
    });
    let now = time.elapsed_secs_f64();
    approaches
        .entries
        .retain(|entity, _| bots.contains(*entity));

    for (entity, destination, ..) in &bots {
        if let Some(region) = destination.interaction {
            let entry = approaches.entries.entry(entity).or_insert(ApproachEntry {
                region,
                since: now,
                selected: None,
                nearby: false,
                rejected: Vec::new(),
                revision: navigation.revision(),
            });
            if entry.region != region {
                *entry = ApproachEntry {
                    region,
                    since: now,
                    selected: None,
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
    let mut ordered: Vec<_> = bots.iter().map(|(entity, ..)| entity).collect();
    ordered.sort_by_key(|entity| {
        (
            approaches
                .entries
                .get(entity)
                .map_or(u64::MAX, |entry| entry.since.to_bits()),
            entity.to_bits(),
        )
    });
    let mut occupancy = FixedSpatialBuckets::new(2.0 * crate::navigation::BODY_RADIUS);
    for (entity, transform, _) in &bodies {
        occupancy.insert(
            transform.translation.truncate(),
            (entity, transform.translation.truncate()),
        );
    }
    let mut claims = FixedSpatialBuckets::new(2.0 * crate::navigation::BODY_RADIUS);
    for entity in ordered {
        let Ok((entity, destination, transform, mut velocity, kind, member, evacuation, yielding)) =
            bots.get_mut(entity)
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
        if let Some(region) = selected_destination.interaction {
            let entry = approaches.entries.get_mut(&entity).unwrap();
            if entry.revision != navigation.revision() {
                entry.rejected.clear();
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
            let free = |point: Vec2| {
                navigation.point_clear(point)
                    && (!nearby
                        || (space_free(entity, point, &occupancy)
                            && space_free(entity, point, &claims)))
            };
            let mut candidates = region.work_candidates(position);
            if let Some(old) = approaches
                .entries
                .get(&entity)
                .and_then(|entry| entry.selected)
            {
                candidates.insert(0, old);
            }
            // A separate position already in reach is useful immediately, even after yielding away from the intended approach.
            if region.contains(position) {
                candidates.insert(0, position);
            }
            let entry = approaches.entries.get(&entity).unwrap();
            let candidates: Vec<_> = candidates
                .into_iter()
                .filter(|point| free(*point) && !entry.rejected.contains(point))
                .collect();
            // Retain a usable commitment, otherwise prefer an immediately clear approach.
            let retained = entry
                .selected
                .filter(|point| candidates.contains(point))
                .filter(|point| {
                    navigation.movement_clear(position, *point)
                        || routes.get(&entity).is_some_and(|route| {
                            route.destination == *point
                                && (route.pending.is_some() || !route.waypoints.is_empty())
                        })
                });
            let selected = retained
                .or_else(|| {
                    candidates
                        .iter()
                        .copied()
                        .find(|point| navigation.movement_clear(position, *point))
                })
                .or_else(|| candidates.first().copied());
            if let Some(point) = selected {
                if nearby {
                    claims.insert(point, (entity, point));
                }
                approaches.entries.get_mut(&entity).unwrap().selected = Some(point);
                selected_destination.xy = point;
                // Route to the selected perimeter position, never the occupied nearest face.
                selected_destination.interaction = None;
                work_ready = nearby
                    && region.contains(position)
                    && space_free(entity, position, &occupancy)
                    && !bodies
                        .get(entity)
                        .is_ok_and(|(_, _, recovering)| recovering)
                    && position.distance(point) <= STOP_THRESHOLD;
                commands.entity(entity).remove::<WaitingForWork>();
            } else {
                if loads.get(entity).is_ok_and(|cargo| cargo.amount > 0)
                    || kind == Some(&super::NanobotType::Defender)
                {
                    let entry = approaches.entries.get_mut(&entity).unwrap();
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
        let direct = navigation.movement_clear(position, destination.xy)
            && (kind != Some(&super::NanobotType::Hauler)
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
            let hauler = kind == Some(&super::NanobotType::Hauler);
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
                    route.current = super::route_following::rejoin_index(
                        position,
                        route.requested_start,
                        &route.waypoints,
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
        let speed = destination.speed.unwrap_or(game_settings.bot_speed);
        let step = route.follower.route_step(
            position,
            &route.waypoints,
            &mut route.current,
            speed,
            kind == Some(&super::NanobotType::Hauler),
            &navigation,
        );
        let next_index = route.follower.target_index(route.current);
        if let Some(next) = route.waypoints.get(next_index) {
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
    occupancy
        .neighbourhood(occupancy.bucket_for_position(position), 1)
        .flat_map(|(_, entries)| entries)
        .all(|(other, other_position)| {
            *other == entity
                || position.distance_squared(*other_position)
                    >= (2.0 * crate::navigation::BODY_RADIUS - 0.001).powi(2)
        })
}
