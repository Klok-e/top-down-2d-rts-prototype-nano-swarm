//! Swarm-owned resource economy data.
//!
//! The simulation owns resources as plain Rust data with no
//! dependency on Bevy rendering or shader storage buffers. Future
//! systems (production facilities, chargers, maintenance) read the
//! same [`ResourceLedger`] to decide whether the swarm can keep
//! going.
//!
//! The contract enforced by this module:
//!   - [`ResourceDeposit`] is a place where physical resources
//!     exist before they are picked up.
//!   - [`Stockpile`] is a place where carried resources are dropped
//!     off. Gather zones depend on a nearby stockpile so workers
//!     have a place to deliver the small loads they carry.
//!   - [`ResourceLedger`] tracks the swarm-wide total for each
//!     [`ResourceKind`] so future systems (e.g. production) can ask
//!     "how many minerals does the swarm have?" without scanning
//!     every entity. It is updated whenever material enters or leaves
//!     a physical buffer (stockpile, terminal hopper, or charger).

use std::collections::HashMap;

use bevy::prelude::{Component, Resource};

/// Kinds of resources the simulation knows about. The first
/// implementation only models [`ResourceKind::Minerals`]; adding
/// more kinds is just a matter of new variants and a wider
/// [`ResourceLedger`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ResourceKind {
    #[default]
    Minerals,
}

impl ResourceKind {
    /// Number of distinct resource kinds. Mirrors the
    /// enum-variant count so callers can size tables.
    pub const COUNT: usize = 1;

    /// All resource kinds in stable order. Useful for tests and
    /// future "iterate every kind" loops.
    pub const ALL: [ResourceKind; Self::COUNT] = [ResourceKind::Minerals];
}

/// Physical resource deposit on the map. The entity carrying this
/// component is the deposit; its world position comes from
/// `Transform`. Workers within `radius` world units of the position
/// can extract from it.
#[derive(Debug, Component, Default, Clone, Copy)]
pub struct ResourceDeposit {
    pub kind: ResourceKind,
    /// How much of `kind` is currently sitting in this deposit.
    pub amount: u32,
    /// Maximum amount this deposit can hold. The simulation does
    /// not auto-respawn deposits in the first implementation; the
    /// `amount` is allowed to go to zero and stay there.
    pub capacity: u32,
    /// Worker reach radius in world units. A worker within
    /// `radius` of the deposit's `Transform` may extract.
    pub radius: f32,
}

impl ResourceDeposit {
    /// True when this deposit still has at least one unit to give.
    pub fn has_work(&self) -> bool {
        self.amount > 0
    }
}

/// Drop-off location for carried resources. Same component shape as
/// [`ResourceDeposit`] but conceptually the inverse: workers dump
/// their load here instead of pulling from it.
#[derive(Debug, Component, Default, Clone, Copy)]
pub struct Stockpile {
    pub kind: ResourceKind,
    pub amount: u32,
    pub capacity: u32,
    pub radius: f32,
}

impl Stockpile {
    /// How many more units this stockpile can accept before it is
    /// full. A worker delivery that would exceed this must be
    /// rejected (the worker will pick a different stockpile or
    /// wait for space to free up).
    pub fn free_space(&self) -> u32 {
        self.capacity.saturating_sub(self.amount)
    }
}

/// Distinguishes a built [`Stockpile`] by its placement role in
/// the swarm's logistics network. The shape and behaviour of the
/// buffer are the same (both carry `ResourceKind` + amount +
/// capacity + radius), but the demand system that creates it
/// and the proximity checks that consult it differ by role:
///
///   - `Source` stockpiles stage gathered resources near
///     `ResourceDeposit`s. The gather worker's "any near usable
///     Source Stockpile" check (issue #23) counts only these.
///   - `Sink` stockpiles live in `Build` cells and stage material
///     for production facilities and future base infrastructure. A
///     hauler can deliver into a sink stockpile (leg 2) and later
///     draw from it to feed a terminal (leg 3). Worker gather
///     delivery targets only source stockpiles so deposit-side flow
///     cannot bypass the tiered logistics chain.
///
/// The marker is independent of [`Stockpile`] so the existing
/// `Stockpile` data shape stays stable: tests and gameplay
/// code that already reason about a bare `Stockpile` continue
/// to work, and the role is a precise filter for the
/// Source-only checks. A `Stockpile` without the marker is
/// treated as `Source` (the legacy default) for the proximity
/// checks, which keeps older hand-spawned stockpiles green
/// while the new Sink Stockpiles are stamped at the planned
/// structure's promotion step.
#[derive(Debug, Component, Default, Clone, Copy, PartialEq, Eq)]
pub enum StockpileRole {
    /// Built from a `PlannedKind::SourceStockpile` plan near a
    /// `ResourceDeposit`. Counts as a "near usable Source
    /// Stockpile" for the gather worker arrive / demand
    /// systems.
    #[default]
    Source,
    /// Built from a `PlannedKind::SinkStockpile` plan inside a
    /// Build cell. Feeds the base; not a Source for the gather
    /// flow.
    Sink,
}

/// Resource totals partitioned by owning swarm.
///
/// `total` and legacy `add`/`remove` expose aggregate/player-compatible
/// behavior while ownership-aware gameplay uses `*_for` methods.
#[derive(Debug, Default, Resource, Clone)]
pub struct ResourceLedger {
    pub totals: HashMap<ResourceKind, u32>,
    swarm_totals: HashMap<crate::nanobot::SwarmId, HashMap<ResourceKind, u32>>,
}

impl ResourceLedger {
    /// Empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Aggregate total across all swarms.
    pub fn total(&self, kind: ResourceKind) -> u32 {
        self.totals.get(&kind).copied().unwrap_or(0)
    }

    /// Total owned by one swarm, including buffers and in-transit cargo.
    pub fn total_for(&self, swarm: crate::nanobot::SwarmId, kind: ResourceKind) -> u32 {
        self.swarm_totals
            .get(&swarm)
            .and_then(|totals| totals.get(&kind))
            .copied()
            .unwrap_or(0)
    }

    /// Legacy player-owned addition.
    pub fn add(&mut self, kind: ResourceKind, amount: u32) {
        self.add_for(crate::nanobot::SwarmId::PLAYER, kind, amount);
    }

    /// Add newly acquired physical resources to one swarm.
    pub fn add_for(&mut self, swarm: crate::nanobot::SwarmId, kind: ResourceKind, amount: u32) {
        let aggregate = self.totals.entry(kind).or_insert(0);
        *aggregate = aggregate.saturating_add(amount);
        let owned = self
            .swarm_totals
            .entry(swarm)
            .or_default()
            .entry(kind)
            .or_insert(0);
        *owned = owned.saturating_add(amount);
    }

    /// Legacy player-owned subtraction.
    pub fn remove(&mut self, kind: ResourceKind, amount: u32) {
        self.remove_for(crate::nanobot::SwarmId::PLAYER, kind, amount);
    }

    /// Remove consumed or destroyed resources from one swarm.
    pub fn remove_for(&mut self, swarm: crate::nanobot::SwarmId, kind: ResourceKind, amount: u32) {
        let removed = self
            .swarm_totals
            .get_mut(&swarm)
            .and_then(|totals| totals.get_mut(&kind))
            .map(|owned| {
                let removed = amount.min(*owned);
                *owned -= removed;
                removed
            })
            .unwrap_or(0);
        if let Some(aggregate) = self.totals.get_mut(&kind) {
            *aggregate = aggregate.saturating_sub(removed);
        }
    }

    /// Number of distinct resource kinds tracked.
    pub fn len(&self) -> usize {
        self.totals.len()
    }

    /// True when no resource kind is tracked.
    pub fn is_empty(&self) -> bool {
        self.totals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_kind_default_is_minerals() {
        assert_eq!(ResourceKind::default(), ResourceKind::Minerals);
    }

    #[test]
    fn resource_kind_all_covers_minerals() {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL.to_vec();
        assert_eq!(kinds.len(), ResourceKind::COUNT);
        assert!(kinds.contains(&ResourceKind::Minerals));
    }

    #[test]
    fn deposit_has_work_only_when_amount_positive() {
        let mut d = ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 0,
            capacity: 10,
            radius: 1.0,
        };
        assert!(!d.has_work());
        d.amount = 1;
        assert!(d.has_work());
        d.amount = 5;
        assert!(d.has_work());
    }

    #[test]
    fn stockpile_free_space_is_capacity_minus_amount_floored() {
        let s = Stockpile {
            kind: ResourceKind::Minerals,
            amount: 7,
            capacity: 10,
            radius: 1.0,
        };
        assert_eq!(s.free_space(), 3);

        let full = Stockpile {
            kind: ResourceKind::Minerals,
            amount: 12,
            capacity: 10,
            radius: 1.0,
        };
        assert_eq!(full.free_space(), 0, "free space never goes negative");
    }

    #[test]
    fn ledger_add_and_remove_track_totals() {
        let mut ledger = ResourceLedger::new();
        assert!(ledger.is_empty());
        assert_eq!(ledger.total(ResourceKind::Minerals), 0);

        ledger.add(ResourceKind::Minerals, 5);
        ledger.add(ResourceKind::Minerals, 3);
        assert_eq!(ledger.total(ResourceKind::Minerals), 8);

        ledger.remove(ResourceKind::Minerals, 2);
        assert_eq!(ledger.total(ResourceKind::Minerals), 6);

        ledger.remove(ResourceKind::Minerals, 100);
        assert_eq!(
            ledger.total(ResourceKind::Minerals),
            0,
            "remove is floored at zero"
        );
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn ledger_add_saturates_at_u32_max() {
        let mut ledger = ResourceLedger::new();
        ledger.add(ResourceKind::Minerals, u32::MAX);
        ledger.add(ResourceKind::Minerals, 1);
        assert_eq!(ledger.total(ResourceKind::Minerals), u32::MAX);
    }

    #[test]
    fn stockpile_role_default_is_source() {
        // The default role is `Source` for back-compat: a
        // hand-spawned `Stockpile` without an explicit role
        // marker still satisfies the gather worker's
        // "near usable Source" check, so pre-existing
        // tests that spawn `Stockpile`s directly keep
        // passing.
        assert_eq!(StockpileRole::default(), StockpileRole::Source);
    }
}
