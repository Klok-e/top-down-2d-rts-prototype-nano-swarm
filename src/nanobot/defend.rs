//! Defend Zone behavior for Defender nanobots.
//!
//! Defenders protect assets and advance through painted Defend intent.
//! Combat uses swarm systems rather than group commands; regional
//! allocation supplies each Defender's current work claim.
//!
//! Regional allocation is the sole source of Defender assignments.
//! This module owns the Defend lifecycle after allocation: movement
//! arrival, supported-cell holding, and local containment.
//!
//! Arrival treats the assigned Defend cell as an area, not a point:
//! a defender counts as arrived once it is within
//! [`DEFEND_IN_CELL_STOP_RADIUS`] of the cell's world center, which
//! keeps it comfortably inside the cell while leaving room for
//! separation-driven cosmetic de-clumping. A holding defender may
//! de-clump inside its assigned cell via separation forces; if it
//! drifts outside the cell the hold system pulls it back to the
//! nearest in-cell point -- this is cosmetic containment, not a new
//! tactical assignment.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::ZONE_BLOCK_SIZE;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::{DirectMovementComponent, Nanobot, SwarmMember};
use crate::nanobot::spatial_pressure::point_in_cell;

/// In-cell arrival and containment stop radius. A defender counts
/// as "arrived" at its assigned Defend cell once it is within this
/// radius of the cell's world center, and a drifted holding
/// defender is pulled back only until it re-enters this radius.
/// Sized at 40% of [`ZONE_BLOCK_SIZE`] so the defender stops
/// comfortably inside the cell (whose half-width is 50% of
/// [`ZONE_BLOCK_SIZE`]) while leaving room for separation-driven
/// cosmetic de-clumping. Larger than
/// [`crate::nanobot::consts::STOP_THRESHOLD`] so the movement
/// system treats it as a real extent rather than falling back to
/// the extent-less sentinel.
pub const DEFEND_IN_CELL_STOP_RADIUS: f32 = ZONE_BLOCK_SIZE * 0.4;

/// Baseline defend-pressure need multiplier applied to every
/// Defend cell. The [`DefendPressure`] hook multiplies this
/// baseline; cells with no explicit entry score at baseline, and
/// a future threat-response system can raise an entry above
/// baseline so enemies inside a painted Defend cell boost that
/// cell's score without changing the scoring architecture or
/// creating defender work outside Defend paint.
pub const DEFEND_PRESSURE_BASELINE: f32 = 1.0;

/// World position of the center of `cell`. Matches
/// `ai::get_world_from_zone` so the regional allocator and test
/// seam agree on the center.
fn cell_center_world(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

/// Marks a Defender as committed to a specific Defend cell. Set by
/// the regional allocator when the defender claims a Defend cell;
/// cleared when the defender transitions into hold state (the
/// `DefendHold` marker takes over) or when the defender is re-routed
/// to a new cell.
///
/// Issue #37: the assigned cell is an area, not a precise point.
/// The [`DirectMovementComponent`] inserted alongside this marker
/// targets the cell center with [`DEFEND_IN_CELL_STOP_RADIUS`] so
/// the defender counts as arrived once it is meaningfully inside the
/// cell. `DefendAssignment` continues to identify the tactical
/// target cell but no longer implies a center-point hold.
#[derive(Debug, Component, Clone, Copy)]
pub struct DefendAssignment {
    pub cell: IVec2,
}

/// Marks a Defender that has arrived at its assigned Defend cell and
/// is now "holding" the position. The defender may carry a
/// [`DirectMovementComponent`] only for cosmetic containment -- if
/// separation forces pushed it outside its assigned cell the hold
/// system re-inserts a DMC to pull it back to the nearest in-cell
/// point. The regional lease stays active for the entire hold
/// duration; the hold system releases it when the cell's paint is
/// erased or regional allocation replaces the claim.
///
/// The hold is "the cell is still painted and the defender stays
/// inside it", not "the defender stands on the exact center". Local
/// cosmetic de-clumping via separation forces is allowed inside the
/// cell; cross-cell movement is allocator-driven.
#[derive(Debug, Component, Clone, Copy)]
pub struct DefendHold {
    pub cell: IVec2,
}

/// Per-cell defend-pressure hook. Each Defend cell's score is
/// multiplied by its pressure value (acting as the cell's need
/// factor); cells with no explicit entry use
/// [`DEFEND_PRESSURE_BASELINE`]. A future threat-response system
/// writes entries above baseline for Defend cells that contain
/// enemies, raising those cells' scores so defenders concentrate
/// where the pressure is, without creating defender work outside
/// Defend paint or changing the scoring architecture.
#[derive(Debug, Default, PartialEq, Resource)]
pub struct DefendPressure {
    map: HashMap<(crate::nanobot::components::SwarmId, IVec2), f32>,
}

impl DefendPressure {
    /// Pressure multiplier for `cell`. Falls back to
    /// [`DEFEND_PRESSURE_BASELINE`] when no entry has been set, so
    /// the initial scoring model is "every Defend cell is equally
    /// pressurised" until a threat system says otherwise.
    pub fn get(&self, cell: IVec2) -> f32 {
        self.get_for(crate::nanobot::components::SwarmId::PLAYER, cell)
    }

    pub fn get_for(&self, swarm: crate::nanobot::components::SwarmId, cell: IVec2) -> f32 {
        self.map
            .get(&(swarm, cell))
            .copied()
            .unwrap_or(DEFEND_PRESSURE_BASELINE)
    }

    /// Set the pressure multiplier for `cell`. Values at or above
    /// [`DEFEND_PRESSURE_BASELINE`] raise the cell's score; values
    /// below baseline lower it. A threat system writes a value
    /// above baseline when enemies occupy the painted Defend cell.
    pub fn set(&mut self, cell: IVec2, value: f32) {
        self.set_for(crate::nanobot::components::SwarmId::PLAYER, cell, value);
    }

    pub fn set_for(&mut self, swarm: crate::nanobot::components::SwarmId, cell: IVec2, value: f32) {
        self.map.insert((swarm, cell), value);
    }

    /// Remove the explicit entry for `cell` so it falls back to
    /// [`DEFEND_PRESSURE_BASELINE`]. Used when the threat that
    /// raised the pressure leaves the cell.
    pub fn remove(&mut self, cell: IVec2) {
        self.map
            .remove(&(crate::nanobot::components::SwarmId::PLAYER, cell));
    }

    /// Reset all explicit pressure before rebuilding the current threat snapshot.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Number of cells with an explicit (non-baseline) pressure
    /// entry. Useful for tests asserting the hook was written.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when no cell has an explicit pressure entry (every cell
    /// scores at baseline).
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Detect a Defender that has arrived at its assigned Defend cell
/// and transition it into the hold state. The trigger is the same
/// as the rest of the simulation: the movement system removes the
/// [`DirectMovementComponent`] when the defender is within
/// [`DEFEND_IN_CELL_STOP_RADIUS`] of the cell center -- i.e. once
/// it is meaningfully inside the cell, not at the exact center.
///
/// The `Without<DefendHold>` filter makes arrival idempotent: the
/// same tick cannot fire twice and a defender already holding does
/// not get a duplicate `DefendHold` marker.
#[allow(clippy::type_complexity)]
pub fn defender_arrive_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    defenders: Query<
        (Entity, &DefendAssignment, &SwarmMember),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<DefendAssignment>,
            Without<DirectMovementComponent>,
            Without<DefendHold>,
        ),
    >,
) {
    for (entity, assignment, member) in &defenders {
        let supported = grid
            .cell(assignment.cell)
            .is_some_and(|cell| cell.visible_to(IntentKind::Defend, member.0));
        if !supported {
            commands.entity(entity).remove::<DefendAssignment>();
            continue;
        }
        commands.entity(entity).remove::<DefendAssignment>();
        commands.entity(entity).insert(DefendHold {
            cell: assignment.cell,
        });
    }
}

/// Keep holding Defenders inside their assigned Defend cell.
///
/// A holding defender may de-clump inside its assigned cell via
/// separation forces (the global separation system runs every tick).
/// The hold system does NOT re-snap the defender to the cell center
/// -- that would cluster every holder on the exact center, the
/// problem issue #37 fixes. Instead it only intervenes when the
/// defender has drifted OUTSIDE its assigned cell: it inserts a
/// containment [`DirectMovementComponent`] aimed at the cell center
/// with [`DEFEND_IN_CELL_STOP_RADIUS`] so the defender stops as soon
/// as it is meaningfully inside again. This is cosmetic containment,
/// not a new tactical assignment: no `DefendAssignment` is inserted.
///
/// The regional lease is released when the cell's paint is erased, ownership
/// changes to another swarm, or regional allocation replaces the claim. The hold
/// marker is removed so the next allocation pass can acquire new work.
#[allow(clippy::type_complexity)]
pub fn defender_hold_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    defenders: Query<
        (Entity, &DefendHold, &Transform, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<DefendHold>,
            Without<DefendAssignment>,
        ),
    >,
) {
    for (entity, hold, transform, nanobot_type, member) in &defenders {
        if *nanobot_type != NanobotType::Defender {
            continue;
        }
        // A hold remains valid only while Defend intent is still visible to the
        // Defender's swarm. Erasure, withdrawal, or hostile capture releases it.
        let still_supported = grid
            .cell(hold.cell)
            .is_some_and(|cell| cell.visible_to(IntentKind::Defend, member.0));
        if !still_supported {
            commands
                .entity(entity)
                .remove::<DefendHold>()
                .remove::<DirectMovementComponent>();
            continue;
        }
        // Cosmetic containment: if the defender drifted outside
        // its assigned cell, pull it back toward the cell center
        // with an in-cell stop radius. A defender still inside
        // its cell is left alone so separation forces can de-clump
        // holders across the cell area.
        let pos = transform.translation.truncate();
        if !point_in_cell(pos, hold.cell) {
            let cell_center = cell_center_world(hold.cell);
            commands.entity(entity).insert(DirectMovementComponent {
                xy: cell_center,
                stop_radius: DEFEND_IN_CELL_STOP_RADIUS,
            });
        }
    }
}

/// Plugin that wires the defender lifecycle into the fixed schedule.
/// The chain runs after movement and regional allocation. Allocation
/// owns assignment; these systems only transition arrival and hold
/// state.
pub struct DefendPlugin;

impl Plugin for DefendPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DefendPressure>();
        app.add_systems(
            FixedUpdate,
            (defender_arrive_system, defender_hold_system)
                .chain()
                .after(crate::nanobot::RegionalAllocationSet::Acquire)
                .after(crate::nanobot::NanobotSimulationSet::Movement),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-resource tests. End-to-end Defender contracts live in
    //! `tests/behavior/defend_zone.rs`.

    use super::*;

    #[test]
    fn defend_pressure_defaults_to_baseline_and_is_overridable() {
        // The hook is the per-cell defend-pressure entry point.
        // With no entry, every cell scores at baseline; a set
        // entry overrides only that cell.
        let pressure = DefendPressure::default();
        assert!(pressure.is_empty());
        assert_eq!(pressure.get(IVec2::new(1, 1)), DEFEND_PRESSURE_BASELINE);
        let mut pressure = pressure;
        pressure.set(IVec2::new(1, 1), 2.5);
        assert_eq!(pressure.len(), 1);
        assert_eq!(pressure.get(IVec2::new(1, 1)), 2.5);
        // Other cells are untouched.
        assert_eq!(pressure.get(IVec2::new(2, 2)), DEFEND_PRESSURE_BASELINE);
        pressure.remove(IVec2::new(1, 1));
        assert!(pressure.is_empty());
        assert_eq!(pressure.get(IVec2::new(1, 1)), DEFEND_PRESSURE_BASELINE);
    }
}
