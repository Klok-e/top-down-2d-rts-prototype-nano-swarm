//! Hauler behaviour and automatic stockpile creation.
//!
//! Haulers move large physical loads between logistics buffers:
//! source stockpiles, sink stockpiles, and terminal consumers
//! (production facilities / chargers). Deposits are worker-only
//! sources under the tiered logistics model; legacy manual hauler
//! assignments can still drain them defensively for tests.

use crate::battle_statistics::{BattleCounters, BattleEvent};
use bevy::prelude::*;

use crate::nanobot::{
    Cargo, InteractionRegion, LogisticsReservation, OwnerSwarm, ProductionFacility,
    SupportCondition,
    charge::Charger,
    components::{DirectMovementComponent, Nanobot, Swarm, SwarmId, SwarmMember},
    logistics_leg::{Destination, DestinationRole},
};
use crate::navigation::{ConnectivityStatus, Navigation, RouteGoal};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole};

/// Maximum units a Hauler can carry in a single trip. The glossary is
/// explicit that Haulers carry "much more" than Workers; this cap is
/// deliberately five times the worker cap so the gap remains visible in
/// the swarm output.
pub const HAULER_CARRY_CAPACITY: u32 = 20;

/// Minerals a Hauler moves after each completed material-work interval.
pub const HAULER_TRANSFER_PER_PULSE: u32 = 1;

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

fn endpoint_is_operational(entity: Entity, conditions: &Query<&SupportCondition>) -> bool {
    conditions
        .get(entity)
        .map_or(true, |condition| condition.is_operational())
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
            commands.entity(entity).insert((
                HaulerLoading,
                Cargo::empty(kind),
                super::MaterialWorkProgress::default(),
            ));
        } else {
            // Restore the final work goal while the source commitment remains active.
            commands
                .entity(entity)
                .insert(region.movement_from(transform.translation.truncate()));
        }
    }
}

/// Drain one mineral from the assigned source after each material-work
/// interval while the hauler is at the source and the load is
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
            &mut super::MaterialWorkProgress,
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
    mut counters: Option<ResMut<BattleCounters>>,
) {
    for (entity, mut cargo, transform, assignment, mut reservation, swarm, mut work) in &mut haulers
    {
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
            work.reset();
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

        if !work.completes_unit() {
            continue;
        }

        if let Ok((mut deposit, _)) = deposits.get_mut(assignment.source) {
            if deposit.amount == 0 {
                finish_reservation(reservation.as_deref_mut(), cargo.amount);
                transition_to_carrying(&mut commands, entity, cargo.amount);
                continue;
            }
            let can_still_carry = target_amount - cargo.amount;
            let actual = HAULER_TRANSFER_PER_PULSE
                .min(deposit.amount)
                .min(can_still_carry);
            cargo.amount += actual;
            deposit.amount -= actual;
            ledger.add_for(swarm.0, deposit.kind, actual);
            if let Some(counters) = counters.as_deref_mut() {
                counters.record(swarm.0, BattleEvent::Gathered(actual));
            }
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
            let actual = HAULER_TRANSFER_PER_PULSE
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
    commands
        .entity(entity)
        .remove::<HaulerLoading>()
        .remove::<super::MaterialWorkProgress>();
    if amount == 0 {
        commands
            .entity(entity)
            .remove::<Cargo>()
            .remove::<HaulerAssignment>()
            .remove::<LogisticsReservation>();
    }
}

/// A loaded Hauler returning minerals to a compatible Stockpile.
#[derive(Component)]
pub struct ReturningCargo;

fn logistics_owner(
    owner: Option<&OwnerSwarm>,
    swarms: &Query<&SwarmId, With<Swarm>>,
) -> Option<SwarmId> {
    owner.and_then(|owner| swarms.get(owner.0).ok()).copied()
}

#[allow(clippy::type_complexity)]
fn source_role(
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
    swarms: &Query<&SwarmId, With<Swarm>>,
) -> Option<StockpileRole> {
    let (_, stockpile, _, role, owner) = stockpiles.get(source).ok()?;
    if stockpile.kind != kind || logistics_owner(owner, swarms) != Some(swarm) {
        return None;
    }
    Some(role.copied().unwrap_or_default())
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
fn valid_destination_region(
    destination: Entity,
    source_role: StockpileRole,
    returning: bool,
    cargo: Cargo,
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
    swarms: &Query<&SwarmId, With<Swarm>>,
    conditions: &Query<&SupportCondition>,
) -> Option<InteractionRegion> {
    if !endpoint_is_operational(destination, conditions) {
        return None;
    }
    let (endpoint, transform) =
        if let Ok((_, stockpile, transform, role, owner)) = stockpiles.get(destination) {
            (
                Destination {
                    kind: stockpile.kind,
                    role: DestinationRole::Stockpile(role.copied().unwrap_or_default()),
                    free_space: stockpile.free_space(),
                    owner: logistics_owner(owner, swarms),
                },
                transform,
            )
        } else if let Ok((_, facility, transform, owner)) = facilities.get(destination) {
            (
                Destination {
                    kind: facility.input_kind,
                    role: DestinationRole::ProductionFacility,
                    free_space: facility.input_free_space(),
                    owner: logistics_owner(owner, swarms),
                },
                transform,
            )
        } else if let Ok((_, charger, transform, owner)) = chargers.get(destination) {
            (
                Destination {
                    kind: charger.kind,
                    role: DestinationRole::Charger,
                    free_space: charger.free_space(),
                    owner: logistics_owner(owner, swarms),
                },
                transform,
            )
        } else {
            return None;
        };
    endpoint
        .can_receive_load(swarm, cargo, source_role, returning, incoming_claims)
        .then(|| InteractionRegion::structure(transform))
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
    swarms: Query<&SwarmId, With<Swarm>>,
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
        let role = source_role(
            assignment.source,
            cargo.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        )
        .unwrap_or_default();
        let position = transform.translation.truncate();
        let endpoint = |candidate, is_return| {
            let incoming = reserved_destination_capacity(&reservations, candidate, Some(entity))
                .saturating_add(
                    same_tick_claims
                        .get(&candidate)
                        .copied()
                        .unwrap_or_default(),
                );
            valid_destination_region(
                candidate,
                role,
                is_return,
                *cargo,
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
            match navigation.query_connectivity(position, RouteGoal::Interaction(current)) {
                ConnectivityStatus::Connected { .. } | ConnectivityStatus::Pending => continue,
                ConnectivityStatus::Unreachable => {}
            }
        }
        let mut best: Option<(bool, f32, Entity, InteractionRegion)> = None;
        let mut pending_destination = false;
        for candidate in stockpiles
            .iter()
            .map(|(e, ..)| e)
            .chain(facilities.iter().map(|(e, ..)| e))
            .chain(chargers.iter().map(|(e, ..)| e))
        {
            let is_stockpile = stockpiles.contains(candidate);
            let ordinary = (role != StockpileRole::Sink || !is_stockpile)
                .then(|| endpoint(candidate, false))
                .flatten();
            let (is_return, target) = if let Some(target) = ordinary {
                (false, target)
            } else if is_stockpile && let Some(target) = endpoint(candidate, true) {
                (true, target)
            } else {
                continue;
            };
            let cost = match navigation.query_connectivity(position, RouteGoal::Interaction(target))
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
            .insert((redirected, target.movement_from(position)));
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
    swarms: Query<&SwarmId, With<Swarm>>,
    reservations: Query<(Entity, &LogisticsReservation)>,
) {
    for (entity, transform, cargo, assignment, swarm_member, returning) in &haulers {
        let role = source_role(
            assignment.source,
            cargo.kind,
            swarm_member.0,
            &stockpiles,
            &swarms,
        )
        .unwrap_or_default();
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
        let Some(sink) = valid_destination_region(
            assignment.sink,
            role,
            returning.is_some(),
            *cargo,
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
        if sink.contains(hauler_pos) {
            continue;
        }
        let movement = sink.movement_from(hauler_pos);
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
            Option<&mut super::MaterialWorkProgress>,
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
    swarms: Query<&SwarmId, With<Swarm>>,
    reservations: Query<(Entity, &LogisticsReservation)>,
) {
    let mut destination_claims = std::collections::HashMap::<Entity, u32>::new();
    for (_, reservation) in &reservations {
        let total = destination_claims
            .entry(reservation.destination)
            .or_default();
        *total = total.saturating_add(reservation.destination_remaining);
    }

    for (entity, transform, mut load, assignment, reservation, swarm_member, returning, work) in
        &mut haulers
    {
        let role = source_role(
            assignment.source,
            load.kind,
            swarm_member.0,
            &stockpiles.as_readonly(),
            &swarms,
        )
        .unwrap_or_default();
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
        let Some(endpoint) = valid_destination_region(
            assignment.sink,
            role,
            returning.is_some(),
            *load,
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
        if !endpoint.contains(transform.translation.truncate()) {
            if let Some(mut work) = work {
                work.reset();
            }
            continue;
        }
        let Some(mut work) = work else {
            commands
                .entity(entity)
                .insert(super::MaterialWorkProgress::default());
            continue;
        };
        if !work.completes_unit() {
            continue;
        }
        let transfer_limit = load.amount.min(HAULER_TRANSFER_PER_PULSE);
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
                .remove::<LogisticsReservation>()
                .remove::<super::MaterialWorkProgress>();
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
