//! Shared physical cargo and Logistics Reservation components.

use bevy::prelude::*;

use crate::{
    nanobot::{Health, Nanobot, SwarmMember},
    resources::{ResourceKind, ResourceLedger},
};

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
) {
    for (entity, health, swarm, cargo) in &dead {
        if health.current > 0 {
            continue;
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
