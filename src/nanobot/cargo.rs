//! Shared physical cargo and Logistics Reservation components.

use crate::battle_statistics::{BattleCounters, BattleEvent};
use bevy::prelude::*;

use crate::{
    nanobot::{Health, Nanobot, SwarmMember},
    resources::{ResourceKind, ResourceLedger},
};

/// Progress toward the next mineral moved by active material work.
///
/// This component exists only while a nanobot is stationary and performing
/// gathering, hauling, or construction. Removing it resets partial progress
/// when the work changes or movement resumes.
#[derive(Debug, Component, Default, Clone, Copy, PartialEq, Eq)]
pub struct MaterialWorkProgress {
    pub ticks_elapsed: u32,
}

impl MaterialWorkProgress {
    /// Advance one fixed tick and report whether one mineral may move.
    pub fn completes_unit(&mut self, material_work_ticks: u32) -> bool {
        self.ticks_elapsed = self.ticks_elapsed.saturating_add(1);
        if self.ticks_elapsed < material_work_ticks {
            return false;
        }
        self.ticks_elapsed = 0;
        true
    }

    /// Discard incomplete work after an interruption.
    pub fn reset(&mut self) {
        self.ticks_elapsed = 0;
    }
}

/// Minerals physically carried by a nanobot.
///
/// Cargo remains owned by the nanobot's swarm while in transit, so changing
/// its location does not change the swarm-wide resource total.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq)]
pub struct Cargo {
    pub kind: ResourceKind,
    pub amount: u32,
}

impl Cargo {
    pub const fn empty(kind: ResourceKind) -> Self {
        Self { kind, amount: 0 }
    }
}

/// Exact source quantity and destination capacity claimed for one movement.
///
/// A reservation coordinates future transfer. Creating or removing one never
/// changes physical resource amounts or swarm ownership.
#[derive(Debug, Component, Clone, Copy, PartialEq, Eq)]
pub struct LogisticsReservation {
    pub source: Entity,
    pub destination: Entity,
    pub kind: ResourceKind,
    pub amount: u32,
    pub source_remaining: u32,
    pub destination_remaining: u32,
}

impl LogisticsReservation {
    pub const fn new(source: Entity, destination: Entity, kind: ResourceKind, amount: u32) -> Self {
        Self {
            source,
            destination,
            kind,
            amount,
            source_remaining: amount,
            destination_remaining: amount,
        }
    }
}

/// Remove dead nanobots after settling physical cargo ownership.
///
/// All gameplay nanobot death paths set `Health.current` to zero and let this
/// system release reservations, remove exact remaining cargo from its swarm,
/// then despawn. Transfers and invalid cancellations never use this path.
#[allow(clippy::type_complexity)]
pub fn nanobot_death_cleanup_system(
    mut commands: Commands,
    dead: Query<(Entity, &Health, &SwarmMember, Option<&Cargo>), (With<Nanobot>, Changed<Health>)>,
    mut ledger: ResMut<ResourceLedger>,
    mut counters: Option<ResMut<BattleCounters>>,
) {
    for (entity, health, swarm, cargo) in &dead {
        if health.current > 0 {
            continue;
        }
        if let Some(counters) = counters.as_deref_mut() {
            counters.record(swarm.0, BattleEvent::Death);
        }
        if let Some(cargo) = cargo {
            ledger.remove_for(swarm.0, cargo.kind, cargo.amount);
        }
        commands
            .entity(entity)
            .remove::<LogisticsReservation>()
            .despawn();
    }
}

/// Apply live logistics limits without changing physical mineral custody.
pub fn reconcile_logistics_tuning(
    world: &mut World,
    old: &crate::tuning::Tuning,
    new: &crate::tuning::Tuning,
) {
    if old.logistics.material_work_ticks != new.logistics.material_work_ticks {
        for mut progress in world.query::<&mut MaterialWorkProgress>().iter_mut(world) {
            progress.ticks_elapsed = crate::tuning::rescale_ticks(
                progress.ticks_elapsed,
                old.logistics.material_work_ticks,
                new.logistics.material_work_ticks,
            )
            .min(new.logistics.material_work_ticks.saturating_sub(1));
        }
    }
    if old.logistics.stockpile_capacity != new.logistics.stockpile_capacity {
        for mut stockpile in world
            .query::<&mut crate::resources::Stockpile>()
            .iter_mut(world)
        {
            stockpile.capacity = new.logistics.stockpile_capacity;
        }
    }
    if old.logistics.corridor_cost_multiplier != new.logistics.corridor_cost_multiplier
        && let Some(mut navigation) = world.get_resource_mut::<crate::navigation::Navigation>()
    {
        navigation.set_corridor_cost_multiplier(new.logistics.corridor_cost_multiplier);
    }

    if old.logistics.worker_carry_capacity == new.logistics.worker_carry_capacity
        && old.logistics.hauler_carry_capacity == new.logistics.hauler_carry_capacity
        && old.logistics.stockpile_capacity == new.logistics.stockpile_capacity
        && old.production.input_capacity == new.production.input_capacity
        && old.charge.charger_capacity == new.charge.charger_capacity
    {
        return;
    }

    let mut free_capacity = std::collections::HashMap::<Entity, u32>::new();
    for (entity, stockpile) in world
        .query::<(Entity, &crate::resources::Stockpile)>()
        .iter(world)
    {
        free_capacity.insert(entity, stockpile.free_space());
    }
    for (entity, facility) in world
        .query::<(Entity, &super::ProductionFacility)>()
        .iter(world)
    {
        free_capacity.insert(entity, facility.input_free_space());
    }
    for (entity, charger) in world.query::<(Entity, &super::Charger)>().iter(world) {
        free_capacity.insert(entity, charger.free_space());
    }
    let mut reservations: Vec<_> = world
        .query::<(
            Entity,
            &LogisticsReservation,
            Option<&Cargo>,
            Option<&super::NanobotType>,
        )>()
        .iter(world)
        .map(|(entity, reservation, cargo, kind)| {
            (entity, *reservation, cargo.copied(), kind.copied())
        })
        .collect();
    // Existing physical loads retain capacity before future pickups.
    reservations.sort_by_key(|(entity, _, cargo, _)| {
        (
            cargo.is_none_or(|cargo| cargo.amount == 0),
            entity.to_bits(),
        )
    });
    for (entity, mut reservation, cargo, kind) in reservations {
        let carried = cargo.map_or(0, |cargo| cargo.amount);
        let carry_capacity = match kind {
            Some(super::NanobotType::Worker) => new.logistics.worker_carry_capacity,
            _ => new.logistics.hauler_carry_capacity,
        };
        let available = free_capacity.entry(reservation.destination).or_default();
        let pickup = reservation
            .source_remaining
            .min(carry_capacity.saturating_sub(carried));
        let desired = carried.saturating_add(pickup);
        let claimed = desired.min(*available);
        *available = available.saturating_sub(claimed);
        reservation.source_remaining = claimed.saturating_sub(carried);
        reservation.destination_remaining = claimed;
        reservation.amount = carried.saturating_add(reservation.source_remaining);
        let mut bot = world.entity_mut(entity);
        if claimed == 0 && carried == 0 {
            bot.remove::<(
                LogisticsReservation,
                Cargo,
                super::GatherAssignment,
                super::ExtractProgress,
                super::HaulerAssignment,
                super::HaulerLoading,
                MaterialWorkProgress,
                super::DirectMovementComponent,
                super::RegionalLease,
            )>();
            bot.insert(super::Commitment::Idle);
            continue;
        }
        bot.insert(reservation);
        if reservation.source_remaining == 0
            && (bot.contains::<super::ExtractProgress>() || bot.contains::<super::HaulerLoading>())
        {
            bot.remove::<(
                super::ExtractProgress,
                super::GatherAssignment,
                super::HaulerLoading,
                MaterialWorkProgress,
            )>();
        }
        if claimed == 0 && carried > 0 {
            bot.remove::<(super::DirectMovementComponent, super::ReturningToStockpile)>();
        }
    }
    if world.contains_resource::<crate::navigation::Navigation>() {
        use bevy::ecs::system::RunSystemOnce;
        world
            .run_system_once(super::gather::worker_gather_reroute_system)
            .expect("worker cargo reconciliation must run");
        world
            .run_system_once(super::haul::hauler_reroute_system)
            .expect("hauler cargo reconciliation must run");
    }
}
