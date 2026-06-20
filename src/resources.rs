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
//!     every entity. It is updated whenever a deposit loses
//!     resources or a stockpile gains them.

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

/// Swarm-wide resource totals. Inserted as a Bevy [`Resource`]
/// so future systems (production, maintenance) can ask "how much
/// of `kind` does the swarm have?" without scanning every entity.
///
/// Maintained by the gather systems: deposits going down and
/// stockpiles going up both flow through the ledger so the totals
/// stay consistent with what is physically in the world.
#[derive(Debug, Default, Resource, Clone)]
pub struct ResourceLedger {
    pub totals: HashMap<ResourceKind, u32>,
}

impl ResourceLedger {
    /// Empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current total for `kind`, or `0` when the kind has no
    /// entry yet.
    pub fn total(&self, kind: ResourceKind) -> u32 {
        self.totals.get(&kind).copied().unwrap_or(0)
    }

    /// Add `amount` units of `kind` to the ledger. Saturates at
    /// `u32::MAX` so a runaway source cannot overflow.
    pub fn add(&mut self, kind: ResourceKind, amount: u32) {
        let entry = self.totals.entry(kind).or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    /// Subtract `amount` units of `kind` from the ledger.
    /// Floored at zero so a delivery that empties the entry does
    /// not underflow.
    pub fn remove(&mut self, kind: ResourceKind, amount: u32) {
        if let Some(total) = self.totals.get_mut(&kind) {
            *total = total.saturating_sub(amount);
        }
    }

    /// Number of distinct resource kinds tracked. Useful for tests
    /// and debug overlays.
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
}
