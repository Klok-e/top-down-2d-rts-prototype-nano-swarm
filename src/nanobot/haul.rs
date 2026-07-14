//! Hauler behaviour and automatic stockpile creation.
//!
//! Haulers move large physical loads between logistics buffers:
//! source stockpiles, sink stockpiles, and terminal consumers
//! (production facilities / chargers). Deposits are worker-only
//! sources under the tiered logistics model; legacy manual hauler
//! assignments can still drain them defensively for tests.

use bevy::prelude::*;

use crate::intent::IntentGrid;
use crate::nanobot::{
    Cargo, LogisticsReservation, NanobotType, OwnerSwarm, ProductionFacility, STOP_THRESHOLD,
    charge::Charger,
    components::{DirectMovementComponent, Nanobot, SwarmId, SwarmMember},
    hauler_route_cost,
    logistics_leg::{
        HaulerContext, StockpileCandidate, TerminalCandidate, pick_logistics_leg_with_cost,
    },
    placement::BUILDING_FOOTPRINT_RADIUS,
    plan_hauler_route,
};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole};

/// Maximum units a Hauler can carry in a single trip. The glossary is
/// explicit that Haulers carry "much more" than Workers; this cap is
/// deliberately five times the worker cap so the gap is visible in the
/// swarm output and obvious in the test math.
pub const HAULER_CARRY_CAPACITY: u32 = 20;

/// Units a Hauler pulls from its source per `app.update()` tick.
/// Four units/tick means a hauler fills the 20-unit load in 5 ticks;
/// large enough that the trip is short relative to the load but
/// small enough that the test can drive the simulation forward with
/// a handful of updates.
pub const HAULER_EXTRACT_PER_TICK: u32 = 4;

/// Units a Hauler transfers into a destination per simulation tick.
pub const HAULER_TRANSFER_PER_TICK: u32 = HAULER_EXTRACT_PER_TICK;

/// Backwards-compatible name for the shared cargo carried by a Hauler.
/// Cargo exists during gradual loading and remains after loading completes.
pub type HaulerLoad = Cargo;

/// Marks a Hauler as committed to a specific `(source, sink)` pair.
/// In normal tiered logistics, `source` is a non-empty stockpile and
/// `sink` is a sink stockpile or terminal consumer. Defensive legacy
/// paths still tolerate deposit sources for hand-seeded assignments.
/// Both are kept on the same component because the hauler commits to
/// the whole trip in the assignment system rather than picking the
/// sink at delivery time.
#[derive(Debug, Component, Clone, Copy)]
pub struct HaulerAssignment {
    pub source: Entity,
    pub sink: Entity,
}

/// Marks a Hauler as standing at its assigned source and loading cargo.
#[derive(Debug, Component, Default, Clone, Copy)]
pub struct HaulerLoading;

/// Stable route for one hauler Logistics Leg.
#[derive(Debug, Component, Clone)]
pub struct HaulerRoute {
    pub waypoints: Vec<Vec2>,
    pub current: usize,
    pub final_stop_radius: f32,
}

impl HaulerRoute {
    pub fn new(waypoints: Vec<Vec2>, final_stop_radius: f32) -> Self {
        Self {
            waypoints,
            current: 0,
            final_stop_radius,
        }
    }

    fn current_waypoint(&self) -> Option<Vec2> {
        self.waypoints.get(self.current).copied()
    }

    fn current_stop_radius(&self) -> f32 {
        if self.current + 1 == self.waypoints.len() {
            self.final_stop_radius
        } else {
            0.0
        }
    }

    fn current_movement(&self) -> Option<DirectMovementComponent> {
        self.current_waypoint().map(|xy| DirectMovementComponent {
            xy,
            stop_radius: self.current_stop_radius(),
        })
    }
}

/// Follow a stable route by issuing the current waypoint as direct movement.
pub fn hauler_route_follow_system(
    mut commands: Commands,
    mut haulers: Query<(
        Entity,
        &Transform,
        &mut HaulerRoute,
        Option<&DirectMovementComponent>,
    )>,
) {
    for (entity, transform, mut route, dmc) in &mut haulers {
        if route.waypoints.is_empty() {
            commands.entity(entity).remove::<HaulerRoute>();
            continue;
        }

        let pos = transform.translation.truncate();
        loop {
            let Some(waypoint) = route.current_waypoint() else {
                commands.entity(entity).remove::<HaulerRoute>();
                break;
            };
            let stop_radius = route.current_stop_radius();
            let threshold = if stop_radius > 0.0 {
                stop_radius.max(STOP_THRESHOLD)
            } else {
                STOP_THRESHOLD
            };
            if pos.distance(waypoint) > threshold {
                if dmc.is_none_or(|dmc| {
                    (dmc.xy - waypoint).length() > 1.0
                        || (dmc.stop_radius - stop_radius).abs() > f32::EPSILON
                }) {
                    commands.entity(entity).insert(DirectMovementComponent {
                        xy: waypoint,
                        stop_radius,
                    });
                }
                break;
            }
            route.current += 1;
        }
    }
}

fn route_waypoints_or_direct(
    start: Vec2,
    end: Vec2,
    grid: &IntentGrid,
    swarm: SwarmId,
) -> Vec<Vec2> {
    plan_hauler_route(start, end, grid, swarm)
        .map(|route| route.waypoints)
        .unwrap_or_else(|| vec![end])
}

pub(crate) fn planned_route_movement(
    start: Vec2,
    end: Vec2,
    grid: &IntentGrid,
    swarm: SwarmId,
    final_stop_radius: f32,
) -> (HaulerRoute, DirectMovementComponent) {
    let route = HaulerRoute::new(
        route_waypoints_or_direct(start, end, grid, swarm),
        final_stop_radius,
    );
    let movement = route.current_movement().unwrap_or(DirectMovementComponent {
        xy: end,
        stop_radius: final_stop_radius,
    });
    (route, movement)
}

/// Convert an optional [`OwnerSwarm`] marker into the concrete [`SwarmId`]
/// used by the pure Logistics Leg picker. A broken owner reference
/// makes the candidate unusable, matching the old `owner_matches`
/// behaviour.
fn candidate_owner(
    owner: Option<&OwnerSwarm>,
    swarms: &Query<&SwarmId>,
) -> Option<Option<SwarmId>> {
    match owner {
        None => Some(None),
        Some(OwnerSwarm(owner_entity)) => swarms.get(*owner_entity).ok().copied().map(Some),
    }
}

/// World position of a hauler source stockpile. A hauler source
/// is always a stockpile under the tiered model, so this is a
/// plain component lookup.
#[allow(clippy::type_complexity)]
fn stockpile_pos(
    entity: Entity,
    stockpiles: &Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
) -> Option<Vec2> {
    stockpiles
        .get(entity)
        .ok()
        .map(|(_, _, t, _, _)| t.translation.truncate())
}

/// Physical extent of a hauler source stockpile, used as the
/// arrival stop radius so the movement system halts on the
/// stockpile's edge (matching the arrive-source guard).
#[allow(clippy::type_complexity)]
fn stockpile_radius_of(
    entity: Entity,
    stockpiles: &Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
) -> f32 {
    stockpiles
        .get(entity)
        .map(|(_, s, _, _, _)| s.radius)
        .unwrap_or(0.0)
}

/// For each idle Hauler with no in-flight transport work, pick a
/// `(source, sink)` pair from the resource economy and head to the
/// source. The hauler keeps a single [`HaulerAssignment`] for the
/// whole trip so the carry-to-sink step does not need to re-select
/// the sink from scratch.
#[allow(clippy::type_complexity)]
pub fn hauler_assignment_system(
    mut commands: Commands,
    haulers: Query<
        (Entity, &Transform, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            With<NanobotType>,
            Without<HaulerAssignment>,
            Without<HaulerLoad>,
            Without<HaulerLoading>,
            Without<DirectMovementComponent>,
        ),
    >,
    stockpiles: Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: Query<(Entity, &ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    chargers: Query<(Entity, &Charger, &Transform, Option<&OwnerSwarm>)>,
    swarms: Query<&SwarmId>,
    grid: Res<IntentGrid>,
) {
    let stockpile_candidates: Vec<StockpileCandidate> = stockpiles
        .iter()
        .filter_map(|(entity, stockpile, transform, role, owner)| {
            let owner = candidate_owner(owner, &swarms)?;
            Some(StockpileCandidate {
                entity,
                pos: transform.translation.truncate(),
                kind: stockpile.kind,
                role: role.copied().unwrap_or(StockpileRole::Source),
                amount: stockpile.amount,
                free_space: stockpile.free_space(),
                owner,
            })
        })
        .collect();
    let mut terminal_candidates: Vec<TerminalCandidate> = facilities
        .iter()
        .filter_map(|(entity, facility, transform, owner)| {
            let owner = candidate_owner(owner, &swarms)?;
            Some(TerminalCandidate::Facility {
                entity,
                pos: transform.translation.truncate(),
                kind: facility.input_kind,
                free_space: facility.input_free_space(),
                owner,
            })
        })
        .collect();
    terminal_candidates.extend(chargers.iter().filter_map(
        |(entity, charger, transform, owner)| {
            let owner = candidate_owner(owner, &swarms)?;
            Some(TerminalCandidate::Charger {
                entity,
                pos: transform.translation.truncate(),
                kind: charger.kind,
                free_space: charger.free_space(),
                owner,
            })
        },
    ));

    for (entity, transform, nanobot_type, swarm_member) in &haulers {
        if *nanobot_type != NanobotType::Hauler {
            continue;
        }
        let hauler_pos = transform.translation.truncate();
        let swarm = swarm_member.0;
        let Some(leg) = pick_logistics_leg_with_cost(
            HaulerContext {
                pos: hauler_pos,
                swarm,
                kind: ResourceKind::Minerals,
                carry_capacity: HAULER_CARRY_CAPACITY,
            },
            &stockpile_candidates,
            &terminal_candidates,
            |from, to| hauler_route_cost(from, to, &grid, swarm),
        ) else {
            continue;
        };
        let source = leg.source;
        let sink = leg.sink;
        let Some(source_pos) = stockpile_pos(source, &stockpiles) else {
            continue;
        };

        // Source-side stop radius: the source stockpile's own
        // physical extent. A hauler source is always a stockpile
        // under the tiered model (deposits are a worker-only
        // source), so the lookup never falls through. Issue #38
        // / ADR-0004: same extent as the hauler-arrive-source
        // guard so the movement system and the arrive system
        // stop on the same edge.
        let source_radius = stockpile_radius_of(source, &stockpiles);

        let (route, movement) =
            planned_route_movement(hauler_pos, source_pos, &grid, swarm, source_radius);

        commands.entity(entity).insert((
            HaulerAssignment { source, sink },
            LogisticsReservation::new(source, sink, ResourceKind::Minerals, leg.amount),
            route,
            movement,
        ));
    }
}

/// Detect a hauler that has arrived at its assigned source and
/// start the loading phase. The arrival threshold is the source's
/// own radius (deposit or stockpile), matching the gather chain.
/// The `Without<HaulerLoading>` filter makes arrival idempotent; the
/// `Without<HaulerLoad>` filter keeps a Carrying hauler from being
/// re-loaded when it happens to be at the source between trips.
#[allow(clippy::type_complexity)]
pub fn hauler_arrive_source_system(
    mut commands: Commands,
    haulers: Query<
        (
            Entity,
            &Transform,
            &HaulerAssignment,
            Option<&HaulerRoute>,
            Option<&LogisticsReservation>,
        ),
        (
            With<Nanobot>,
            With<HaulerAssignment>,
            Without<DirectMovementComponent>,
            Without<HaulerLoading>,
            Without<HaulerLoad>,
        ),
    >,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    stockpiles: Query<(&Stockpile, &Transform)>,
    chargers: Query<(&Charger, &Transform)>,
) {
    for (entity, transform, assignment, route, reservation) in &haulers {
        let (source_pos, source_radius) = if let Ok((d, t)) = deposits.get(assignment.source) {
            (t.translation.truncate(), d.radius)
        } else if let Ok((s, t)) = stockpiles.get(assignment.source) {
            (t.translation.truncate(), s.radius)
        } else if let Ok((c, t)) = chargers.get(assignment.source) {
            (t.translation.truncate(), c.radius)
        } else {
            // Source entity disappeared; drop the assignment and
            // let a later tick reassign.
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<LogisticsReservation>()
                .remove::<HaulerRoute>();
            continue;
        };
        if transform.translation.truncate().distance(source_pos) <= source_radius {
            let kind = reservation
                .map(|reservation| reservation.kind)
                .unwrap_or(ResourceKind::Minerals);
            commands
                .entity(entity)
                .insert((HaulerLoading, Cargo::empty(kind)))
                .remove::<HaulerRoute>();
        } else if route.is_some() {
            // The route follower owns movement restoration while a
            // route is active.
        } else {
            // `ProgressChecker` can remove `DirectMovementComponent`
            // before true arrival when congestion leaves the hauler
            // below the progress threshold. Keep the source-side
            // commitment alive and restore movement instead of
            // marooning the hauler with `HaulerAssignment` and no
            // velocity.
            commands.entity(entity).insert(DirectMovementComponent {
                xy: source_pos,
                stop_radius: source_radius,
            });
        }
    }
}

/// Drain `HAULER_EXTRACT_PER_TICK` units from the assigned source
/// every tick while the hauler is at the source and the load is
/// not full. When the load is full or the source empties (or
/// disappears), transition the hauler to Carrying.
#[allow(clippy::type_complexity)]
pub fn hauler_load_system(
    mut commands: Commands,
    mut haulers: Query<
        (
            Entity,
            &mut Cargo,
            &HaulerAssignment,
            Option<&mut LogisticsReservation>,
            &SwarmMember,
        ),
        (With<Nanobot>, With<HaulerLoading>),
    >,
    mut deposits: Query<&mut ResourceDeposit>,
    mut source_stockpiles: Query<&mut Stockpile>,
    source_chargers: Query<&mut Charger>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, mut cargo, assignment, mut reservation, swarm) in &mut haulers {
        let target_amount = reservation
            .as_ref()
            .map(|reservation| reservation.amount)
            .unwrap_or(HAULER_CARRY_CAPACITY);
        let finish_reservation = |reservation: Option<&mut LogisticsReservation>, carried| {
            if let Some(reservation) = reservation {
                reservation.source_remaining = 0;
                reservation.destination_remaining = carried;
            }
        };
        if cargo.amount >= target_amount {
            finish_reservation(reservation.as_deref_mut(), cargo.amount);
            transition_to_carrying(&mut commands, entity, cargo.amount);
            continue;
        }

        if let Ok(mut deposit) = deposits.get_mut(assignment.source) {
            if deposit.amount == 0 {
                finish_reservation(reservation.as_deref_mut(), cargo.amount);
                transition_to_carrying(&mut commands, entity, cargo.amount);
                continue;
            }
            let can_still_carry = target_amount - cargo.amount;
            let actual = HAULER_EXTRACT_PER_TICK
                .min(deposit.amount)
                .min(can_still_carry);
            cargo.amount += actual;
            deposit.amount -= actual;
            ledger.add_for(swarm.0, deposit.kind, actual);
            if let Some(reservation) = reservation.as_deref_mut() {
                reservation.source_remaining = reservation.source_remaining.saturating_sub(actual);
            }
            continue;
        }

        if let Ok(mut stockpile) = source_stockpiles.get_mut(assignment.source) {
            if stockpile.amount == 0 {
                finish_reservation(reservation.as_deref_mut(), cargo.amount);
                transition_to_carrying(&mut commands, entity, cargo.amount);
                continue;
            }
            let can_still_carry = target_amount - cargo.amount;
            let actual = HAULER_EXTRACT_PER_TICK
                .min(stockpile.amount)
                .min(can_still_carry);
            cargo.amount += actual;
            stockpile.amount -= actual;
            if let Some(reservation) = reservation.as_deref_mut() {
                reservation.source_remaining = reservation.source_remaining.saturating_sub(actual);
            }
            continue;
        }

        if source_chargers.get(assignment.source).is_ok() {
            finish_reservation(reservation.as_deref_mut(), cargo.amount);
            transition_to_carrying(&mut commands, entity, 0);
            continue;
        }

        finish_reservation(reservation.as_deref_mut(), cargo.amount);
        transition_to_carrying(&mut commands, entity, cargo.amount);
    }
}

fn transition_to_carrying(commands: &mut Commands, entity: Entity, amount: u32) {
    commands.entity(entity).remove::<HaulerLoading>();
    if amount == 0 {
        commands
            .entity(entity)
            .remove::<Cargo>()
            .remove::<HaulerAssignment>()
            .remove::<LogisticsReservation>();
    }
}

/// Snapshot of a validated destination endpoint.
#[derive(Debug, Clone, Copy)]
struct SinkEndpointSnapshot {
    pos: Vec2,
    radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HaulSourceTier {
    Source,
    Sink,
}

fn owner_is_swarm(owner: Option<&OwnerSwarm>, swarms: &Query<&SwarmId>, swarm: SwarmId) -> bool {
    owner
        .and_then(|owner| swarms.get(owner.0).ok())
        .is_some_and(|owner| *owner == swarm)
}

#[allow(clippy::type_complexity)]
fn source_tier(
    source: Entity,
    kind: ResourceKind,
    swarm: SwarmId,
    stockpiles: &Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    swarms: &Query<&SwarmId>,
) -> Option<HaulSourceTier> {
    let (_, stockpile, _, role, owner) = stockpiles.get(source).ok()?;
    if stockpile.kind != kind || !owner_is_swarm(owner, swarms, swarm) {
        return None;
    }
    match role.copied().unwrap_or(StockpileRole::Source) {
        StockpileRole::Source => Some(HaulSourceTier::Source),
        StockpileRole::Sink => Some(HaulSourceTier::Sink),
    }
}

fn reserved_destination_capacity(
    reservations: &Query<(Entity, &LogisticsReservation)>,
    destination: Entity,
    excluded: Option<Entity>,
) -> u32 {
    reservations
        .iter()
        .filter(|(entity, reservation)| {
            Some(*entity) != excluded && reservation.destination == destination
        })
        .map(|(_, reservation)| reservation.destination_remaining)
        .sum()
}

fn reservation_covers_destination(
    reservation: Option<&LogisticsReservation>,
    destination: Entity,
    amount: u32,
) -> bool {
    reservation.is_none_or(|reservation| {
        reservation.destination == destination && reservation.destination_remaining >= amount
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn valid_destination_snapshot(
    destination: Entity,
    tier: HaulSourceTier,
    kind: ResourceKind,
    amount: u32,
    swarm: SwarmId,
    incoming_claims: u32,
    stockpiles: &Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: &Query<(Entity, &ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    chargers: &Query<(Entity, &Charger, &Transform, Option<&OwnerSwarm>)>,
    swarms: &Query<&SwarmId>,
) -> Option<SinkEndpointSnapshot> {
    if let Ok((_, stockpile, transform, role, owner)) = stockpiles.get(destination) {
        return (stockpile.kind == kind
            && role.copied().unwrap_or(StockpileRole::Source) == StockpileRole::Sink
            && owner_is_swarm(owner, swarms, swarm)
            && stockpile.free_space().saturating_sub(incoming_claims) >= amount)
            .then_some(SinkEndpointSnapshot {
                pos: transform.translation.truncate(),
                radius: stockpile.radius,
            });
    }
    if tier != HaulSourceTier::Sink {
        return None;
    }
    if let Ok((_, facility, transform, owner)) = facilities.get(destination) {
        return (facility.input_kind == kind
            && owner_is_swarm(owner, swarms, swarm)
            && facility.input_free_space().saturating_sub(incoming_claims) >= amount)
            .then_some(SinkEndpointSnapshot {
                pos: transform.translation.truncate(),
                radius: BUILDING_FOOTPRINT_RADIUS,
            });
    }
    if let Ok((_, charger, transform, owner)) = chargers.get(destination) {
        return (charger.kind == kind
            && owner_is_swarm(owner, swarms, swarm)
            && charger.free_space().saturating_sub(incoming_claims) >= amount)
            .then_some(SinkEndpointSnapshot {
                pos: transform.translation.truncate(),
                radius: charger.radius,
            });
    }
    None
}

/// Redirect loaded cargo when its destination is no longer valid.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn hauler_reroute_system(
    mut commands: Commands,
    mut haulers: Query<
        (
            Entity,
            &Transform,
            &Cargo,
            &mut HaulerAssignment,
            &SwarmMember,
            Option<&LogisticsReservation>,
        ),
        (With<Nanobot>, Without<HaulerLoading>),
    >,
    stockpiles: Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: Query<(Entity, &ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    chargers: Query<(Entity, &Charger, &Transform, Option<&OwnerSwarm>)>,
    swarms: Query<&SwarmId>,
    reservations: Query<(Entity, &LogisticsReservation)>,
    grid: Res<IntentGrid>,
) {
    let mut same_tick_claims = std::collections::HashMap::<Entity, u32>::new();
    for (entity, transform, cargo, mut assignment, swarm_member, reservation) in &mut haulers {
        if cargo.amount == 0 {
            continue;
        }
        let Some(tier) = source_tier(
            assignment.source,
            cargo.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        ) else {
            release_destination_claim(&mut commands, entity, reservation);
            continue;
        };
        let current_incoming =
            reserved_destination_capacity(&reservations, assignment.sink, Some(entity))
                .saturating_add(
                    same_tick_claims
                        .get(&assignment.sink)
                        .copied()
                        .unwrap_or_default(),
                );
        let current_claim_valid =
            reservation_covers_destination(reservation, assignment.sink, cargo.amount);
        if current_claim_valid
            && valid_destination_snapshot(
                assignment.sink,
                tier,
                cargo.kind,
                cargo.amount,
                swarm_member.0,
                current_incoming,
                &stockpiles,
                &facilities,
                &chargers,
                &swarms,
            )
            .is_some()
        {
            continue;
        }
        let keep_away_from_old_destination = reservation.is_some_and(|reservation| {
            reservation.destination == assignment.sink && reservation.destination_remaining > 0
        });

        let hauler_pos = transform.translation.truncate();
        let terminal = (tier == HaulSourceTier::Sink)
            .then(|| {
                facilities
                    .iter()
                    .filter_map(|(candidate, _, transform, _)| {
                        if candidate == assignment.sink && keep_away_from_old_destination {
                            return None;
                        }
                        let incoming =
                            reserved_destination_capacity(&reservations, candidate, Some(entity))
                                .saturating_add(
                                    same_tick_claims
                                        .get(&candidate)
                                        .copied()
                                        .unwrap_or_default(),
                                );
                        let endpoint = valid_destination_snapshot(
                            candidate,
                            tier,
                            cargo.kind,
                            cargo.amount,
                            swarm_member.0,
                            incoming,
                            &stockpiles,
                            &facilities,
                            &chargers,
                            &swarms,
                        )?;
                        Some((
                            hauler_pos.distance(transform.translation.truncate()),
                            candidate,
                            endpoint,
                        ))
                    })
                    .chain(chargers.iter().filter_map(|(candidate, _, transform, _)| {
                        if candidate == assignment.sink && keep_away_from_old_destination {
                            return None;
                        }
                        let incoming =
                            reserved_destination_capacity(&reservations, candidate, Some(entity))
                                .saturating_add(
                                    same_tick_claims
                                        .get(&candidate)
                                        .copied()
                                        .unwrap_or_default(),
                                );
                        let endpoint = valid_destination_snapshot(
                            candidate,
                            tier,
                            cargo.kind,
                            cargo.amount,
                            swarm_member.0,
                            incoming,
                            &stockpiles,
                            &facilities,
                            &chargers,
                            &swarms,
                        )?;
                        Some((
                            hauler_pos.distance(transform.translation.truncate()),
                            candidate,
                            endpoint,
                        ))
                    }))
                    .min_by(|left, right| {
                        left.0
                            .total_cmp(&right.0)
                            .then_with(|| left.1.to_bits().cmp(&right.1.to_bits()))
                    })
            })
            .flatten();
        let fallback = stockpiles
            .iter()
            .filter_map(|(candidate, _, transform, _, _)| {
                if candidate == assignment.sink && keep_away_from_old_destination {
                    return None;
                }
                let incoming =
                    reserved_destination_capacity(&reservations, candidate, Some(entity))
                        .saturating_add(
                            same_tick_claims
                                .get(&candidate)
                                .copied()
                                .unwrap_or_default(),
                        );
                let endpoint = valid_destination_snapshot(
                    candidate,
                    tier,
                    cargo.kind,
                    cargo.amount,
                    swarm_member.0,
                    incoming,
                    &stockpiles,
                    &facilities,
                    &chargers,
                    &swarms,
                )?;
                Some((
                    candidate != assignment.source,
                    hauler_pos.distance(transform.translation.truncate()),
                    candidate,
                    endpoint,
                ))
            })
            .min_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| left.1.total_cmp(&right.1))
                    .then_with(|| left.2.to_bits().cmp(&right.2.to_bits()))
            })
            .map(|(_, distance, candidate, endpoint)| (distance, candidate, endpoint));

        let Some((_, destination, endpoint)) = terminal.or(fallback) else {
            release_destination_claim(&mut commands, entity, reservation);
            continue;
        };
        *same_tick_claims.entry(destination).or_default() += cargo.amount;
        assignment.sink = destination;
        let mut redirected = reservation.copied().unwrap_or_else(|| {
            LogisticsReservation::new(assignment.source, destination, cargo.kind, cargo.amount)
        });
        redirected.destination = destination;
        redirected.destination_remaining = cargo.amount;
        let (route, movement) = planned_route_movement(
            hauler_pos,
            endpoint.pos,
            &grid,
            swarm_member.0,
            endpoint.radius,
        );
        commands
            .entity(entity)
            .insert((redirected, route, movement));
    }
}

fn release_destination_claim(
    commands: &mut Commands,
    entity: Entity,
    reservation: Option<&LogisticsReservation>,
) {
    if let Some(reservation) = reservation {
        let mut released = *reservation;
        released.destination_remaining = 0;
        commands.entity(entity).insert(released);
    }
    commands
        .entity(entity)
        .remove::<DirectMovementComponent>()
        .remove::<HaulerRoute>();
}

/// Route loaded haulers only after revalidating the committed destination.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn hauler_carry_assign_system(
    mut commands: Commands,
    haulers: Query<
        (
            Entity,
            &Transform,
            &Cargo,
            &HaulerAssignment,
            &SwarmMember,
            Option<&HaulerRoute>,
        ),
        (
            With<Nanobot>,
            With<Cargo>,
            Without<DirectMovementComponent>,
            Without<HaulerLoading>,
        ),
    >,
    stockpiles: Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: Query<(Entity, &ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    chargers: Query<(Entity, &Charger, &Transform, Option<&OwnerSwarm>)>,
    swarms: Query<&SwarmId>,
    reservations: Query<(Entity, &LogisticsReservation)>,
    grid: Res<IntentGrid>,
) {
    for (entity, transform, cargo, assignment, swarm_member, route) in &haulers {
        let Some(tier) = source_tier(
            assignment.source,
            cargo.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        ) else {
            continue;
        };
        if !reservation_covers_destination(
            reservations
                .get(entity)
                .ok()
                .map(|(_, reservation)| reservation),
            assignment.sink,
            cargo.amount,
        ) {
            continue;
        }
        let incoming = reserved_destination_capacity(&reservations, assignment.sink, Some(entity));
        let Some(sink) = valid_destination_snapshot(
            assignment.sink,
            tier,
            cargo.kind,
            cargo.amount,
            swarm_member.0,
            incoming,
            &stockpiles,
            &facilities,
            &chargers,
            &swarms,
        ) else {
            continue;
        };
        let hauler_pos = transform.translation.truncate();
        if hauler_pos.distance(sink.pos) <= sink.radius || route.is_some() {
            continue;
        }
        let (route, movement) =
            planned_route_movement(hauler_pos, sink.pos, &grid, swarm_member.0, sink.radius);
        commands.entity(entity).insert((route, movement));
    }
}

/// Unload only into a destination that remains valid for cargo's source tier.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn hauler_delivery_system(
    mut commands: Commands,
    mut haulers: Query<
        (
            Entity,
            &Transform,
            &mut Cargo,
            &HaulerAssignment,
            Option<&LogisticsReservation>,
            &SwarmMember,
        ),
        (
            With<Nanobot>,
            With<Cargo>,
            With<HaulerAssignment>,
            Without<DirectMovementComponent>,
            Without<HaulerLoading>,
        ),
    >,
    stockpiles: Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: Query<(Entity, &ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    chargers: Query<(Entity, &Charger, &Transform, Option<&OwnerSwarm>)>,
    swarms: Query<&SwarmId>,
    reservations: Query<(Entity, &LogisticsReservation)>,
) {
    for (entity, transform, mut load, assignment, reservation, swarm_member) in &mut haulers {
        let Some(tier) = source_tier(
            assignment.source,
            load.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        ) else {
            continue;
        };
        if !reservation_covers_destination(reservation, assignment.sink, load.amount) {
            continue;
        }
        let incoming = reserved_destination_capacity(&reservations, assignment.sink, Some(entity));
        let Some(endpoint) = valid_destination_snapshot(
            assignment.sink,
            tier,
            load.kind,
            load.amount,
            swarm_member.0,
            incoming,
            &stockpiles,
            &facilities,
            &chargers,
            &swarms,
        ) else {
            continue;
        };
        if transform.translation.truncate().distance(endpoint.pos) > endpoint.radius {
            continue;
        }
        let transfer_limit = load.amount.min(HAULER_TRANSFER_PER_TICK);
        let actual = if let Ok((_, stockpile, _, _, _)) = stockpiles.get(assignment.sink) {
            let actual = transfer_limit.min(stockpile.free_space());
            let mut updated = *stockpile;
            updated.amount += actual;
            commands.entity(assignment.sink).insert(updated);
            actual
        } else if let Ok((_, facility, _, _)) = facilities.get(assignment.sink) {
            let actual = transfer_limit.min(facility.input_free_space());
            let mut updated = facility.clone();
            updated.input_amount += actual;
            commands.entity(assignment.sink).insert(updated);
            actual
        } else if let Ok((_, charger, _, _)) = chargers.get(assignment.sink) {
            let actual = transfer_limit.min(charger.free_space());
            let mut updated = *charger;
            updated.amount += actual;
            commands.entity(assignment.sink).insert(updated);
            actual
        } else {
            0
        };
        if actual == 0 {
            continue;
        }
        load.amount -= actual;
        if load.amount == 0 {
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<Cargo>()
                .remove::<LogisticsReservation>()
                .remove::<HaulerRoute>();
        } else if let Some(reservation) = reservation {
            let mut updated = *reservation;
            updated.destination_remaining = updated.destination_remaining.saturating_sub(actual);
            commands.entity(entity).insert(updated);
        }
    }
}

/// Plugin that wires the hauler systems into the Update
/// schedule. The chain runs after `move_velocity_system` so the
/// movement system has already pruned arrived bots (which is the
/// trigger the arrive and delivery systems wait for).
///
/// Note: the previous "instant stockpile" auto-creation system
/// (issue #8's `stockpile_auto_creation_system`) was removed in
/// issue #26. Sink Stockpiles now emerge through the planned
/// structure lifecycle in [`PlannedStructurePlugin`], where a
/// Build-painted cell plans a `PlannedKind::SinkStockpile` that
/// a Worker builds into a completed `Stockpile` stamped with
/// [`crate::resources::StockpileRole::Sink`]. Source Stockpiles
/// follow the same lifecycle but live in Gather cells (see
/// [`crate::nanobot::gather::source_stockpile_demand_system`]).
/// There is no longer a path that spawns a completed `Stockpile`
/// directly from Build paint.
pub struct HaulPlugin;

impl Plugin for HaulPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                hauler_arrive_source_system,
                hauler_load_system,
                hauler_reroute_system,
                hauler_carry_assign_system,
                hauler_delivery_system,
                hauler_route_follow_system,
            )
                .chain()
                .after(crate::nanobot::RegionalAllocationSet::Acquire)
                .after(crate::nanobot::move_velocity_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! (transport, capacity, auto-creation) are covered by
    //! `tests/stockpile_and_haul_behavior.rs`.

    use super::*;
    use crate::nanobot::{gather::WORKER_CARRY_CAPACITY, planned::DEFAULT_STOCKPILE_CAPACITY};

    #[test]
    fn hauler_carry_capacity_is_much_larger_than_worker_capacity() {
        // The glossary says haulers carry "much more" than
        // workers. 5x is the floor that keeps the gap visible in
        // test math and swarm behaviour. A const block turns the
        // compile-time check into a real invariant and dodges
        // clippy's "assertion on a constant" lint.
        const { assert!(HAULER_CARRY_CAPACITY >= 5 * WORKER_CARRY_CAPACITY) };
    }

    #[test]
    fn hauler_carry_capacity_is_one_tenth_of_stockpile_capacity() {
        // One full hauler load is one tenth of a completed
        // Source or Sink Stockpile buffer.
        const { assert!(HAULER_CARRY_CAPACITY * 10 == DEFAULT_STOCKPILE_CAPACITY) };
    }

    #[test]
    fn hauler_extract_per_tick_divides_capacity() {
        // The hauler's load fills in a small whole number of
        // ticks. This keeps test math simple and avoids a
        // "stuck at the source for an awkward number of ticks"
        // pattern. Const block keeps the check compile-time so
        // a future tuning pass that breaks the invariant fails
        // the build, not just a test run.
        const { assert!(HAULER_CARRY_CAPACITY.is_multiple_of(HAULER_EXTRACT_PER_TICK)) };
    }
}
