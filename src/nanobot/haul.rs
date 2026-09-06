//! Hauler behaviour and automatic stockpile creation.
//!
//! Haulers move large physical loads between logistics buffers:
//! source stockpiles, sink stockpiles, and terminal consumers
//! (production facilities / chargers). Deposits are worker-only
//! sources under the tiered logistics model; legacy manual hauler
//! assignments can still drain them defensively for tests.

use bevy::prelude::*;

use crate::nanobot::{
    Cargo, InteractionRegion, LogisticsReservation, NanobotType, OwnerSwarm, ProductionFacility,
    SupportCondition,
    charge::Charger,
    components::{DirectMovementComponent, Nanobot, SwarmId, SwarmMember},
    logistics_leg::{
        HaulerContext, StockpileCandidate, TerminalCandidate, pick_logistics_leg_with_cost,
    },
};
use crate::navigation::{ConnectivityStatus, Navigation, RouteGoal};
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

fn endpoint_is_operational(entity: Entity, conditions: &Query<&SupportCondition>) -> bool {
    conditions
        .get(entity)
        .map_or(true, |condition| condition.is_operational())
}

/// For each idle Hauler with no in-flight transport work, pick a
/// `(source, sink)` pair from the resource economy and head to the
/// source. The hauler keeps a single [`HaulerAssignment`] for the
/// whole trip so the carry-to-sink step does not need to re-select
/// the sink from scratch.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
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
    conditions: Query<&SupportCondition>,
    swarms: Query<&SwarmId>,
    navigation: Res<Navigation>,
) {
    let stockpile_candidates: Vec<StockpileCandidate> = stockpiles
        .iter()
        .filter_map(|(entity, stockpile, transform, role, owner)| {
            if !endpoint_is_operational(entity, &conditions) {
                return None;
            }
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
            if !endpoint_is_operational(entity, &conditions) {
                return None;
            }
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
            if !endpoint_is_operational(entity, &conditions) {
                return None;
            }
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
            |from, to| {
                let region_at = |position: Vec2| {
                    stockpiles
                        .iter()
                        .map(|(_, _, t, _, _)| t)
                        .chain(facilities.iter().map(|(_, _, t, _)| t))
                        .chain(chargers.iter().map(|(_, _, t, _)| t))
                        .find(|t| t.translation.truncate() == position)
                        .map(InteractionRegion::structure)
                };
                let start = if from == hauler_pos {
                    from
                } else if let Some(region) = region_at(from) {
                    match navigation.query_connectivity(hauler_pos, RouteGoal::Interaction(region))
                    {
                        ConnectivityStatus::Connected { endpoint } => endpoint,
                        ConnectivityStatus::Pending => {
                            return f32::INFINITY;
                        }
                        ConnectivityStatus::Unreachable => return f32::INFINITY,
                    }
                } else {
                    from
                };
                let outcome = if let Some(region) = region_at(to) {
                    navigation.query_connectivity(start, RouteGoal::Interaction(region))
                } else {
                    navigation.query_connectivity(start, RouteGoal::Point(to))
                };
                match outcome {
                    ConnectivityStatus::Connected { endpoint } => start.distance(endpoint),
                    ConnectivityStatus::Pending => f32::INFINITY,
                    ConnectivityStatus::Unreachable => f32::INFINITY,
                }
            },
        ) else {
            continue;
        };
        let source = leg.source;
        let sink = leg.sink;
        let Ok((_, _, source_transform, _, _)) = stockpiles.get(source) else {
            continue;
        };
        let movement = InteractionRegion::structure(source_transform).movement_from(hauler_pos);

        commands.entity(entity).insert((
            HaulerAssignment { source, sink },
            LogisticsReservation::new(source, sink, ResourceKind::Minerals, leg.amount),
            movement,
        ));
    }
}

/// Detect a hauler that has arrived at its assigned source and
/// start loading from its exterior interaction region.
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
            Option<&LogisticsReservation>,
        ),
        (
            Without<super::WorkBlocked>,
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
    conditions: Query<&SupportCondition>,
) {
    for (entity, transform, assignment, reservation) in &haulers {
        if !endpoint_is_operational(assignment.source, &conditions) {
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<LogisticsReservation>();
            continue;
        }
        let region = if let Ok((d, t)) = deposits.get(assignment.source) {
            InteractionRegion::deposit(t, d.radius)
        } else if let Ok((_, t)) = stockpiles.get(assignment.source) {
            InteractionRegion::structure(t)
        } else if let Ok((_, t)) = chargers.get(assignment.source) {
            InteractionRegion::structure(t)
        } else {
            // Source entity disappeared; drop the assignment and
            // let a later tick reassign.
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<LogisticsReservation>();
            continue;
        };
        if region.contains(transform.translation.truncate()) {
            let kind = reservation
                .map(|reservation| reservation.kind)
                .unwrap_or(ResourceKind::Minerals);
            commands
                .entity(entity)
                .insert((HaulerLoading, Cargo::empty(kind)));
        } else {
            // Restore the final work goal while the source commitment remains active.
            commands
                .entity(entity)
                .insert(region.movement_from(transform.translation.truncate()));
        }
    }
}

/// Drain `HAULER_EXTRACT_PER_TICK` units from the assigned source
/// every tick while the hauler is at the source and the load is
/// not full. When the load is full or the source empties (or
/// disappears), transition the hauler to Carrying.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn hauler_load_system(
    mut commands: Commands,
    navigation: Res<Navigation>,
    mut haulers: Query<
        (
            Entity,
            &mut Cargo,
            &Transform,
            &HaulerAssignment,
            Option<&mut LogisticsReservation>,
            &SwarmMember,
        ),
        (
            Without<super::WorkBlocked>,
            With<Nanobot>,
            With<HaulerLoading>,
        ),
    >,
    mut deposits: Query<(&mut ResourceDeposit, &Transform)>,
    mut source_stockpiles: Query<(&mut Stockpile, &Transform)>,
    source_chargers: Query<(&Charger, &Transform)>,
    conditions: Query<&SupportCondition>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, mut cargo, transform, assignment, mut reservation, swarm) in &mut haulers {
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

        let region = if let Ok((deposit, target)) = deposits.get(assignment.source) {
            Some(InteractionRegion::deposit(target, deposit.radius))
        } else if let Ok((_, target)) = source_stockpiles.get(assignment.source) {
            Some(InteractionRegion::structure(target))
        } else {
            source_chargers
                .get(assignment.source)
                .ok()
                .map(|(_, target)| InteractionRegion::structure(target))
        };
        if let Some(region) = region
            && !region.contains(transform.translation.truncate())
        {
            if matches!(
                navigation.query_connectivity(
                    transform.translation.truncate(),
                    RouteGoal::Interaction(region)
                ),
                ConnectivityStatus::Unreachable
            ) {
                finish_reservation(reservation.as_deref_mut(), cargo.amount);
                commands.entity(entity).remove::<DirectMovementComponent>();
                transition_to_carrying(&mut commands, entity, cargo.amount);
            } else {
                commands
                    .entity(entity)
                    .insert(region.movement_from(transform.translation.truncate()));
            }
            continue;
        }

        if let Ok((mut deposit, _)) = deposits.get_mut(assignment.source) {
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

        if !endpoint_is_operational(assignment.source, &conditions) {
            finish_reservation(reservation.as_deref_mut(), cargo.amount);
            transition_to_carrying(&mut commands, entity, cargo.amount);
            continue;
        }
        if let Ok((mut stockpile, _)) = source_stockpiles.get_mut(assignment.source) {
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
            transition_to_carrying(&mut commands, entity, cargo.amount);
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
    region: InteractionRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HaulSourceTier {
    Source,
    Sink,
}

/// A loaded Hauler returning minerals to a compatible Stockpile.
#[derive(Component)]
pub struct ReturningCargo;

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
    returning: bool,
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
    conditions: &Query<&SupportCondition>,
) -> Option<SinkEndpointSnapshot> {
    if !endpoint_is_operational(destination, conditions) {
        return None;
    }
    if let Ok((_, stockpile, transform, role, owner)) = stockpiles.get(destination) {
        return (stockpile.kind == kind
            && (returning
                || role.copied().unwrap_or(StockpileRole::Source) == StockpileRole::Sink)
            && owner_is_swarm(owner, swarms, swarm)
            && stockpile.free_space().saturating_sub(incoming_claims) >= amount)
            .then_some(SinkEndpointSnapshot {
                region: InteractionRegion::structure(transform),
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
                region: InteractionRegion::structure(transform),
            });
    }
    if let Ok((_, charger, transform, owner)) = chargers.get(destination) {
        return (charger.kind == kind
            && owner_is_swarm(owner, swarms, swarm)
            && charger.free_space().saturating_sub(incoming_claims) >= amount)
            .then_some(SinkEndpointSnapshot {
                region: InteractionRegion::structure(transform),
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
            Option<&ReturningCargo>,
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
    conditions: Query<&SupportCondition>,
    swarms: Query<&SwarmId>,
    reservations: Query<(Entity, &LogisticsReservation)>,
    navigation: Res<Navigation>,
) {
    let mut same_tick_claims = std::collections::HashMap::<Entity, u32>::new();
    for (entity, transform, cargo, mut assignment, swarm_member, reservation, returning) in
        &mut haulers
    {
        if cargo.amount == 0 {
            continue;
        }
        let tier = source_tier(
            assignment.source,
            cargo.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        )
        .unwrap_or(HaulSourceTier::Source);
        let position = transform.translation.truncate();
        let endpoint = |candidate, is_return| {
            let incoming = reserved_destination_capacity(&reservations, candidate, Some(entity))
                .saturating_add(
                    same_tick_claims
                        .get(&candidate)
                        .copied()
                        .unwrap_or_default(),
                );
            valid_destination_snapshot(
                candidate,
                tier,
                is_return,
                cargo.kind,
                cargo.amount,
                swarm_member.0,
                incoming,
                &stockpiles,
                &facilities,
                &chargers,
                &swarms,
                &conditions,
            )
        };
        if reservation_covers_destination(reservation, assignment.sink, cargo.amount)
            && let Some(current) = endpoint(assignment.sink, returning.is_some())
        {
            match navigation.query_connectivity(position, RouteGoal::Interaction(current.region)) {
                ConnectivityStatus::Connected { .. } | ConnectivityStatus::Pending => continue,
                ConnectivityStatus::Unreachable => {}
            }
        }
        let mut best: Option<(bool, f32, Entity, SinkEndpointSnapshot)> = None;
        let mut pending_destination = false;
        for candidate in stockpiles
            .iter()
            .map(|(e, ..)| e)
            .chain(facilities.iter().map(|(e, ..)| e))
            .chain(chargers.iter().map(|(e, ..)| e))
        {
            let is_stockpile = stockpiles.contains(candidate);
            let ordinary = (tier != HaulSourceTier::Sink || !is_stockpile)
                .then(|| endpoint(candidate, false))
                .flatten();
            let (is_return, target) = if let Some(target) = ordinary {
                (false, target)
            } else if is_stockpile && let Some(target) = endpoint(candidate, true) {
                (true, target)
            } else {
                continue;
            };
            let cost = match navigation
                .query_connectivity(position, RouteGoal::Interaction(target.region))
            {
                ConnectivityStatus::Connected { endpoint } => position.distance(endpoint),
                ConnectivityStatus::Pending => {
                    pending_destination |= !is_return;
                    continue;
                }
                ConnectivityStatus::Unreachable => continue,
            };
            if best
                .as_ref()
                .is_none_or(|(old_return, old_cost, old_entity, _)| {
                    (is_return, cost, candidate.to_bits())
                        < (*old_return, *old_cost, old_entity.to_bits())
                })
            {
                best = Some((is_return, cost, candidate, target));
            }
        }
        if pending_destination && best.as_ref().is_none_or(|(is_return, ..)| *is_return) {
            release_destination_claim(&mut commands, entity, reservation);
            continue;
        }
        let Some((is_return, _, destination, target)) = best else {
            release_destination_claim(&mut commands, entity, reservation);
            continue;
        };
        *same_tick_claims.entry(destination).or_default() += cargo.amount;
        assignment.sink = destination;
        let mut redirected =
            LogisticsReservation::new(assignment.source, destination, cargo.kind, cargo.amount);
        redirected.source_remaining = 0;
        commands
            .entity(entity)
            .insert((redirected, target.region.movement_from(position)));
        if is_return {
            commands.entity(entity).insert(ReturningCargo);
        } else {
            commands.entity(entity).remove::<ReturningCargo>();
        }
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
    commands.entity(entity).remove::<DirectMovementComponent>();
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
            Option<&ReturningCargo>,
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
    conditions: Query<&SupportCondition>,
    swarms: Query<&SwarmId>,
    reservations: Query<(Entity, &LogisticsReservation)>,
) {
    for (entity, transform, cargo, assignment, swarm_member, returning) in &haulers {
        let tier = source_tier(
            assignment.source,
            cargo.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        )
        .unwrap_or(HaulSourceTier::Source);
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
            returning.is_some(),
            cargo.kind,
            cargo.amount,
            swarm_member.0,
            incoming,
            &stockpiles,
            &facilities,
            &chargers,
            &swarms,
            &conditions,
        ) else {
            continue;
        };
        let hauler_pos = transform.translation.truncate();
        if sink.region.contains(hauler_pos) {
            continue;
        }
        let movement = sink.region.movement_from(hauler_pos);
        commands.entity(entity).insert(movement);
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
            Option<&ReturningCargo>,
        ),
        (
            Without<super::WorkBlocked>,
            With<Nanobot>,
            With<Cargo>,
            With<HaulerAssignment>,
            Without<DirectMovementComponent>,
            Without<HaulerLoading>,
        ),
    >,
    mut stockpiles: Query<(
        Entity,
        &mut Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    mut facilities: Query<(
        Entity,
        &mut ProductionFacility,
        &Transform,
        Option<&OwnerSwarm>,
    )>,
    mut chargers: Query<(Entity, &mut Charger, &Transform, Option<&OwnerSwarm>)>,
    conditions: Query<&SupportCondition>,
    swarms: Query<&SwarmId>,
    reservations: Query<(Entity, &LogisticsReservation)>,
) {
    let mut destination_claims = std::collections::HashMap::<Entity, u32>::new();
    for (_, reservation) in &reservations {
        let total = destination_claims
            .entry(reservation.destination)
            .or_default();
        *total = total.saturating_add(reservation.destination_remaining);
    }

    for (entity, transform, mut load, assignment, reservation, swarm_member, returning) in
        &mut haulers
    {
        let tier = source_tier(
            assignment.source,
            load.kind,
            swarm_member.0,
            &stockpiles.as_readonly(),
            &swarms,
        )
        .unwrap_or(HaulSourceTier::Source);
        if !reservation_covers_destination(reservation, assignment.sink, load.amount) {
            continue;
        }
        let own_claim = reservation
            .filter(|reservation| reservation.destination == assignment.sink)
            .map(|reservation| reservation.destination_remaining)
            .unwrap_or_default();
        let incoming = destination_claims
            .get(&assignment.sink)
            .copied()
            .unwrap_or_default()
            .saturating_sub(own_claim);
        let Some(endpoint) = valid_destination_snapshot(
            assignment.sink,
            tier,
            returning.is_some(),
            load.kind,
            load.amount,
            swarm_member.0,
            incoming,
            &stockpiles.as_readonly(),
            &facilities.as_readonly(),
            &chargers.as_readonly(),
            &swarms,
            &conditions,
        ) else {
            continue;
        };
        if !endpoint.region.contains(transform.translation.truncate()) {
            continue;
        }
        let transfer_limit = load.amount.min(HAULER_TRANSFER_PER_TICK);
        let actual = if let Ok((_, mut stockpile, _, _, _)) = stockpiles.get_mut(assignment.sink) {
            let actual = transfer_limit.min(stockpile.free_space());
            stockpile.amount += actual;
            actual
        } else if let Ok((_, mut facility, _, _)) = facilities.get_mut(assignment.sink) {
            let actual = transfer_limit.min(facility.input_free_space());
            facility.input_amount += actual;
            actual
        } else if let Ok((_, mut charger, _, _)) = chargers.get_mut(assignment.sink) {
            let actual = transfer_limit.min(charger.free_space());
            charger.amount += actual;
            actual
        } else {
            0
        };
        if actual == 0 {
            continue;
        }
        load.amount -= actual;
        if let Some(reservation) = reservation {
            let remaining = if load.amount == 0 {
                0
            } else {
                reservation.destination_remaining.saturating_sub(actual)
            };
            let released = reservation.destination_remaining.saturating_sub(remaining);
            let total = destination_claims
                .entry(reservation.destination)
                .or_default();
            *total = total.saturating_sub(released);
        }
        if load.amount == 0 {
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<Cargo>()
                .remove::<ReturningCargo>()
                .remove::<LogisticsReservation>();
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
            FixedUpdate,
            (
                hauler_arrive_source_system,
                hauler_load_system,
                hauler_reroute_system,
                hauler_carry_assign_system,
                hauler_delivery_system,
            )
                .chain()
                .after(crate::nanobot::RegionalAllocationSet::Acquire)
                .after(crate::nanobot::NanobotSimulationSet::Movement),
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
