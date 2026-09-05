use super::{
    DirectMovementComponent, ProgressChecker, STOP_THRESHOLD, TrafficYield, VelocityComponent,
};
use crate::game_settings::GameSettings;
use crate::spatial::FixedSpatialBuckets;
use bevy::prelude::*;

/// A loaded bot waiting for a separate position; yielding does not reset queue age.
#[derive(Component, Debug)]
pub struct WaitingForWork;

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
    tick: u64,
    entries: std::collections::HashMap<Entity, (super::InteractionRegion, u64, Option<Vec2>)>,
}

pub struct ActiveRoute {
    destination: Vec2,
    stop_radius: f32,
    interaction: Option<super::InteractionRegion>,
    waypoints: Vec<Vec2>,
    current: usize,
    revision: u64,
    pending: Option<crate::navigation::RouteRequestId>,
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
    grid: Res<crate::intent::IntentGrid>,
    game_settings: Res<GameSettings>,
    navigation: Res<crate::navigation::Navigation>,
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
                    .remove::<(RemainingTravel, WaitingForWork)>();
            }
            false
        }
    });
    approaches.tick += 1;
    approaches
        .entries
        .retain(|entity, _| bots.contains(*entity));
    let tick = approaches.tick;
    for (entity, destination, ..) in &bots {
        if let Some(region) = destination.interaction {
            let entry = approaches
                .entries
                .entry(entity)
                .or_insert((region, tick, None));
            if entry.0 != region {
                *entry = (region, tick, None);
            }
        } else {
            approaches.entries.remove(&entity);
        }
    }
    let mut ordered: Vec<_> = bots.iter().map(|(entity, ..)| entity).collect();
    ordered.sort_by_key(|entity| {
        (
            approaches.entries.get(entity).map_or(tick, |entry| entry.1),
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
            let free = |point: Vec2| {
                navigation.point_clear(point)
                    && space_free(entity, point, &occupancy)
                    && space_free(entity, point, &claims)
            };
            let mut candidates = region.work_candidates(position);
            if region.contains(position) {
                candidates.insert(0, position);
            }
            if let Some(old) = approaches.entries.get(&entity).and_then(|entry| entry.2) {
                candidates.insert(0, old);
            }
            let swarm = member.map_or(super::SwarmId::PLAYER, |member| member.0);
            let mut selected = None;
            let mut pending = false;
            for candidate in candidates.into_iter().filter(|point| free(*point)) {
                if navigation.segment_clear(position, candidate)
                    || routes.get(&entity).is_some_and(|route| {
                        route.destination == candidate
                            && route.revision == navigation.revision()
                            && !route.waypoints.is_empty()
                    })
                {
                    selected = Some(candidate);
                    break;
                }
                match navigation.query_point(
                    position,
                    candidate,
                    &grid,
                    swarm,
                    kind == Some(&super::NanobotType::Hauler),
                ) {
                    RouteStatus::Found(_) => {
                        selected = Some(candidate);
                        break;
                    }
                    RouteStatus::Pending => {
                        // Keep the exact probe goal stable while yielding changes the start.
                        approaches.entries.get_mut(&entity).unwrap().2 = Some(candidate);
                        pending = true;
                        break;
                    }
                    RouteStatus::Unreachable => {}
                }
            }
            if let Some(point) = selected {
                claims.insert(point, (entity, point));
                approaches.entries.get_mut(&entity).unwrap().2 = Some(point);
                selected_destination.xy = point;
                // Route to the selected perimeter position, never the occupied nearest face.
                selected_destination.interaction = None;
                work_ready = region.contains(position)
                    && space_free(entity, position, &occupancy)
                    && !bodies
                        .get(entity)
                        .is_ok_and(|(_, _, recovering)| recovering)
                    && position.distance(point) <= STOP_THRESHOLD;
                commands.entity(entity).remove::<WaitingForWork>();
            } else {
                if !pending {
                    if loads.get(entity).is_ok_and(|cargo| cargo.amount > 0)
                        || kind == Some(&super::NanobotType::Defender)
                    {
                        commands.entity(entity).insert(WaitingForWork);
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
                            )>();
                    }
                }
                continue;
            }
        } else {
            commands.entity(entity).remove::<WaitingForWork>();
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
                .remove::<DirectMovementComponent>()
                .remove::<ProgressChecker>();
            if let Some(route) = routes.remove(&entity)
                && let Some(id) = route.pending
            {
                navigation.cancel(id);
            }
            continue;
        }
        let needs_route = routes.get(&entity).is_none_or(|route| {
            (route.pending.is_none()
                && route.destination != destination.xy
                && (original.interaction.is_some()
                    || route.destination.distance(destination.xy) > crate::navigation::CELL_WIDTH
                    || route.current >= route.waypoints.len()))
                || route.stop_radius != stop
                || route.interaction != destination.interaction
                || (route.waypoints.is_empty() && route.revision != navigation.revision())
                || yielding.is_none()
                    && route
                        .waypoints
                        .get(route.current)
                        .is_some_and(|next| !navigation.segment_clear(position, *next))
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
            let pending = Some(navigation.request(position, goal, swarm, hauler, priority));
            let (waypoints, current) = routes
                .get(&entity)
                .filter(|route| {
                    route.interaction == destination.interaction
                        && route
                            .waypoints
                            .get(route.current)
                            .is_some_and(|next| navigation.segment_clear(position, *next))
                })
                .map_or_else(
                    || (Vec::new(), 0),
                    |route| (route.waypoints.clone(), route.current),
                );
            routes.insert(
                entity,
                ActiveRoute {
                    destination: destination.xy,
                    stop_radius: stop,
                    waypoints,
                    interaction: destination.interaction,
                    current,
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
                    route.current = 0;
                    route.pending = None;
                    navigation.cancel(id);
                }
                RouteStatus::Unreachable => {
                    route.waypoints.clear();
                    route.current = 0;
                    route.pending = None;
                    navigation.cancel(id);
                }
            }
        }
        while route
            .waypoints
            .get(route.current)
            .is_some_and(|point| position.distance(*point) < 0.01)
        {
            route.current += 1;
        }
        if let Some(next) = route.waypoints.get(route.current) {
            let remaining = position.distance(*next)
                + route.waypoints[route.current..]
                    .windows(2)
                    .map(|segment| segment[0].distance(segment[1]))
                    .sum::<f32>();
            commands.entity(entity).insert(RemainingTravel(remaining));
            if yielding.is_some() {
                commands.entity(entity).insert(TrafficYield {
                    rejoin: Some(*next),
                });
            }
            let delta = *next - position;
            let speed = destination.speed.unwrap_or(game_settings.bot_speed);
            velocity.value += delta.normalize_or_zero() * speed.min(delta.length());
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
