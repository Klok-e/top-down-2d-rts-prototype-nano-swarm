//! Defend Zone behavior for Defender nanobots.
//!
//! Issue #13 contract: Defenders protect assets in Defend Zones and
//! advance when Defend Zone intent is painted into enemy territory.
//! This is the initial attack/advance behavior; no separate Attack
//! Zone is added. Combat uses swarm systems rather than group
//! commands -- every Defender is an autonomous agent that picks its
//! own work from the global Defend intent.
//!
//! Issue #37 contract: Defenders spread across painted Defend cells
//! using spatial-pressure scoring instead of clustering at cell
//! centers. See `docs/adr/0007-defender-spatial-pressure.md`. The
//! scoring model layers on top of the global intent candidate
//! scoring (type fit, paint strength, distance, commitment):
//!
//! - **paint strength** raises both the cell's attraction and its
//!   desired occupancy (capacity) -- a strongly painted cell can
//!   absorb more defenders before crowding significantly reduces
//!   its score;
//! - **physical density** (every nanobot in the candidate cell)
//!   plus **defender reservations** (the `(cell, Defend)` soft
//!   work slot) combine into a single soft crowding penalty that
//!   never hard-rejects a cell;
//! - **defend pressure** (per-cell hook, defaulting to baseline)
//!   scales the cell's need so future enemy-in-cell pressure can
//!   raise the score without changing the scoring architecture;
//! - the scoring defender's **own body and reservation are
//!   excluded** so a holding defender re-scoring its own cell is
//!   not over-penalised by itself.
//!
//! State machine carried on the defender by marker components:
//!
//! ```text
//!   Idle -> (assignment system) -> Moving (DefendAssignment + DMC)
//!   Moving -> (arrive system)   -> Holding (DefendAssignment + DefendHold)
//!   Holding -> (assignment system, hysteresis) -> Moving (new DefendAssignment)
//! ```
//!
//! "Enemy territory" is defined as a Defend cell whose Chebyshev
//! distance from the Swarm's cell is greater than
//! [`DEFEND_HOME_RADIUS_CELLS`]. Cells inside the radius are
//! "friendly territory" and defenders holding there are guarding the
//! swarm; cells outside are the frontier the swarm is pushing into.
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

use crate::intent::{IntentCell, IntentGrid, IntentKind, PAINT_STRENGTH_CAP};
use crate::nanobot::autonomy::{Commitment, IntentCandidate, NanobotType, SoftWorkSlots};
use crate::nanobot::charge::{ChargerAssignment, ChargerProgress};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, SwarmMember};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::spatial_pressure::{
    cell_density_system, crowding_factor, paint_occupancy_capacity, point_in_cell, CellDensity,
};
use crate::ZONE_BLOCK_SIZE;

/// Number of cells around the Swarm that count as "friendly
/// territory". A Defend cell at Chebyshev distance greater than this
/// from the Swarm's cell is "enemy territory" and triggers the
/// advance behavior. Tuned to be small enough that the test grid
/// (8x8) has both friendly and enemy cells relative to a Swarm at
/// (0, 0), and large enough that a single Defend cell painted at the
/// Swarm origin is unambiguously friendly.
pub const DEFEND_HOME_RADIUS_CELLS: i32 = 1;

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

/// Hysteresis margin for holding-defender retargeting. A holding
/// defender re-scores Defend cells every tick but only retargets
/// when another cell's score beats its current cell's score by
/// this fraction (i.e. `candidate > current * (1 + margin)`). The
/// margin prevents defenders from oscillating between two nearly
/// equally attractive cells and keeps cross-cell spreading an
/// assignment-driven decision rather than uncontrolled drift.
/// Erased current paint is exempt: when the held cell's Defend
/// paint is gone its score is zero, so any remaining candidate
/// clears the margin immediately.
pub const DEFEND_RETARGET_HYSTERESIS: f32 = 0.25;

/// Baseline defend-pressure need multiplier applied to every
/// Defend cell. The [`DefendPressure`] hook multiplies this
/// baseline; cells with no explicit entry score at baseline, and
/// a future threat-response system can raise an entry above
/// baseline so enemies inside a painted Defend cell boost that
/// cell's score without changing the scoring architecture or
/// creating defender work outside Defend paint.
pub const DEFEND_PRESSURE_BASELINE: f32 = 1.0;

/// True when `cell` is in "enemy territory" relative to
/// `swarm_cell`: the Chebyshev distance between the two cells
/// exceeds `home_radius_cells`. Inside the radius the cell is
/// friendly territory and defenders there are guarding the swarm;
/// outside it is the frontier and defenders are advancing.
///
/// Chebyshev distance (king-move) is the natural grid-cell distance
/// because a Defend cell is a square zone: any cell within
/// `home_radius_cells` king-moves of the swarm is "in range" of the
/// swarm's defensive umbrella. Using Manhattan distance would
/// over-count diagonal cells as far away and produce a diamond
/// shape; Chebyshev produces a square that matches the zone grid.
pub fn is_enemy_territory(cell: IVec2, swarm_cell: IVec2, home_radius_cells: i32) -> bool {
    let dx = (cell.x - swarm_cell.x).abs();
    let dy = (cell.y - swarm_cell.y).abs();
    dx.max(dy) > home_radius_cells
}

/// World position of the center of `cell`. Matches
/// `ai::get_world_from_zone` so the assignment system and the test
/// seam agree on the center.
fn cell_center_world(cell: IVec2) -> Vec2 {
    Vec2::new(
        (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
        (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
    )
}

/// Marks a Defender as committed to a specific Defend cell. Set by
/// the assignment system when the defender picks a Defend candidate;
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
/// point. The soft work slot stays occupied for the entire hold
/// duration; the hold system releases it when the cell's paint is
/// erased or the assignment system re-routes the defender.
///
/// The hold is "the cell is still painted and the defender stays
/// inside it", not "the defender stands on the exact center". Local
/// cosmetic de-clumping via separation forces is allowed inside the
/// cell; cross-cell movement is assignment-driven.
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
#[derive(Debug, Default, Resource)]
pub struct DefendPressure {
    map: HashMap<IVec2, f32>,
}

impl DefendPressure {
    /// Pressure multiplier for `cell`. Falls back to
    /// [`DEFEND_PRESSURE_BASELINE`] when no entry has been set, so
    /// the initial scoring model is "every Defend cell is equally
    /// pressurised" until a threat system says otherwise.
    pub fn get(&self, cell: IVec2) -> f32 {
        self.map
            .get(&cell)
            .copied()
            .unwrap_or(DEFEND_PRESSURE_BASELINE)
    }

    /// Set the pressure multiplier for `cell`. Values at or above
    /// [`DEFEND_PRESSURE_BASELINE`] raise the cell's score; values
    /// below baseline lower it. A threat system writes a value
    /// above baseline when enemies occupy the painted Defend cell.
    pub fn set(&mut self, cell: IVec2, value: f32) {
        self.map.insert(cell, value);
    }

    /// Remove the explicit entry for `cell` so it falls back to
    /// [`DEFEND_PRESSURE_BASELINE`]. Used when the threat that
    /// raised the pressure leaves the cell.
    pub fn remove(&mut self, cell: IVec2) {
        self.map.remove(&cell);
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

/// Information about the defender being scored, used to exclude its
/// own body and reservation from candidate crowding. A holding
/// defender's body sits in its held cell and its reservation sits
/// on the `(held_cell, Defend)` soft work slot; without excluding
/// both, re-scoring its own cell would over-penalise the cell it is
/// correctly holding.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefendSelfExclusion {
    /// Cell the defender is physically standing in (its body).
    pub physical_cell: IVec2,
    /// Cell the defender holds a reservation for (its assigned or
    /// held Defend cell), if any. `None` for an idle defender that
    /// has not yet been assigned.
    pub reserved_cell: Option<IVec2>,
}

/// Score one Defend cell for a defender, pure over the
/// spatial-pressure factors. Mirrors the global
/// [`crate::nanobot::autonomy::score_intent`] shape (type fit *
/// paint * need * distance * crowding * commitment) but swaps the
/// generic soft-slot crowding for paint-driven capacity crowding
/// over the cell's resolved occupancy. `physical_density` and
/// `reservations` are expected to already exclude the scoring
/// defender (see [`DefendSelfExclusion`]); this function does not
/// re-exclude.
#[allow(clippy::too_many_arguments)]
fn score_defend_cell(
    commitment: Commitment,
    nanobot_pos: Vec2,
    candidate_cell: IVec2,
    paint_strength: u8,
    defend_pressure: f32,
    physical_density: u32,
    reservations: u32,
    cell_size: f32,
) -> f32 {
    // Type fit is the multiplicative base: a Defender fits Defend
    // (1.0). The caller iterates only Defend cells, but keeping
    // the factor makes the scoring shape explicit and matches the
    // generic scorer.
    let type_fit = NanobotType::Defender.fit_for(IntentKind::Defend);

    // Paint strength is a positive linear term normalised by the
    // cap. Stronger paint raises both this attraction term and the
    // capacity term below, so a strongly painted cell is both more
    // attractive and more tolerant of crowding.
    let paint_norm = paint_strength as f32 / PAINT_STRENGTH_CAP as f32;

    let candidate_pos = Vec2::new(
        (candidate_cell.x as f32 + 0.5) * cell_size,
        (candidate_cell.y as f32 + 0.5) * cell_size,
    );
    let raw_distance = nanobot_pos.distance(candidate_pos);
    let distance_penalty = 1.0 / (1.0 + raw_distance / cell_size.max(1.0));

    // Crowding combines physical density and reservations into one
    // soft penalty against the paint-driven capacity. Never zero.
    let capacity = paint_occupancy_capacity(paint_strength);
    let occupancy = physical_density + reservations;
    let crowding = crowding_factor(occupancy, capacity);

    let reassess = commitment.reassess_factor();
    let need = defend_pressure.max(0.0);

    type_fit * paint_norm * need * distance_penalty * crowding * reassess
}

/// Resolve the per-cell scoring factors for one Defend cell,
/// excluding the scoring defender's own body and reservation. Shared
/// by [`best_defend_candidate`] (which picks the max across all
/// cells) and the assignment system's hysteresis comparison (which
/// scores the held cell specifically). Returning the resolved
/// factors keeps the exclusion logic in one place. Paint strength
/// is read from the grid by the caller, so it is not part of the
/// resolved tuple.
#[allow(clippy::type_complexity)]
fn resolve_defend_factors(
    cell: IVec2,
    slots: &SoftWorkSlots,
    density: &CellDensity,
    pressure: &DefendPressure,
    exclusion: DefendSelfExclusion,
) -> (f32, u32, u32) {
    let physical_raw = density.density(cell);
    let physical = if cell == exclusion.physical_cell {
        physical_raw.saturating_sub(1)
    } else {
        physical_raw
    };
    let reservations_raw = slots.occupied(cell, IntentKind::Defend);
    let reservations = if exclusion.reserved_cell == Some(cell) {
        reservations_raw.saturating_sub(1)
    } else {
        reservations_raw
    };
    let pressure_val = pressure.get(cell);
    (pressure_val, physical, reservations)
}

/// Score every visible owned Defend cell for one defender and return
/// the highest-scoring candidate. This is the Defend-specific
/// counterpart to [`crate::nanobot::autonomy::best_candidate`]: it
/// layers paint-driven capacity crowding, physical density, and the
/// defend-pressure hook on top of the global scoring shape. The
/// scoring defender's own body and reservation are excluded via
/// `exclusion` so a holding defender re-scoring its own cell is not
/// over-penalised by itself.
///
/// Returns [`None`] when no visible owned Defend cell exists; the
/// caller decides what to do (a holding defender in this case will
/// be released by the hold system on the next tick when its paint
/// is gone).
#[allow(clippy::too_many_arguments)]
pub fn best_defend_candidate(
    grid: &IntentGrid,
    commitment: Commitment,
    nanobot_pos: Vec2,
    slots: &SoftWorkSlots,
    density: &CellDensity,
    pressure: &DefendPressure,
    cell_size: f32,
    swarm: crate::nanobot::components::SwarmId,
    exclusion: DefendSelfExclusion,
) -> Option<IntentCandidate> {
    let mut best: Option<IntentCandidate> = None;
    for (cell, intent_cell) in grid.iter_cells() {
        if !intent_cell.has(IntentKind::Defend) {
            continue;
        }
        if !intent_cell.visible_to(IntentKind::Defend, swarm) {
            continue;
        }
        let strength = intent_cell.strength(IntentKind::Defend);
        let (pressure_val, physical, reservations) =
            resolve_defend_factors(cell, slots, density, pressure, exclusion);
        let score = score_defend_cell(
            commitment,
            nanobot_pos,
            cell,
            strength,
            pressure_val,
            physical,
            reservations,
            cell_size,
        );
        if best.is_none_or(|c| score > c.score) {
            let candidate_pos = Vec2::new(
                (cell.x as f32 + 0.5) * cell_size,
                (cell.y as f32 + 0.5) * cell_size,
            );
            best = Some(IntentCandidate {
                cell,
                kind: IntentKind::Defend,
                score,
                paint_strength: strength,
                need: pressure_val,
                distance: nanobot_pos.distance(candidate_pos),
                // slot_count is repurposed to carry the resolved
                // occupancy (physical + reservations, self
                // excluded) so debug callers see the crowding the
                // scorer actually used.
                slot_count: physical + reservations,
            });
        }
    }

    best
}

/// Score a specific Defend cell for a defender using the same
/// spatial-pressure model as [`best_defend_candidate`]. Used by the
/// assignment system's hysteresis check to compare the candidate
/// against the defender's currently held cell. Returns `None` when
/// the cell is no longer a painted, visible Defend cell (its score
/// is effectively zero, so any remaining candidate clears
/// hysteresis).
#[allow(clippy::too_many_arguments)]
fn score_specific_defend_cell(
    grid: &IntentGrid,
    commitment: Commitment,
    nanobot_pos: Vec2,
    cell: IVec2,
    slots: &SoftWorkSlots,
    density: &CellDensity,
    pressure: &DefendPressure,
    cell_size: f32,
    swarm: crate::nanobot::components::SwarmId,
    exclusion: DefendSelfExclusion,
) -> Option<f32> {
    let intent_cell: &IntentCell = grid.cell(cell)?;
    if !intent_cell.has(IntentKind::Defend) {
        return None;
    }
    if !intent_cell.visible_to(IntentKind::Defend, swarm) {
        return None;
    }
    let strength = intent_cell.strength(IntentKind::Defend);
    let (pressure_val, physical, reservations) =
        resolve_defend_factors(cell, slots, density, pressure, exclusion);
    Some(score_defend_cell(
        commitment,
        nanobot_pos,
        cell,
        strength,
        pressure_val,
        physical,
        reservations,
        cell_size,
    ))
}

/// For each Defender that is idle OR holding a cell, score the
/// Defend intent globally with spatial pressure and (re)assign the
/// defender to the best-scoring cell.
///
/// **Idle defenders** are assigned the best-scoring Defend cell
/// outright: the cell's soft work slot is occupied and a
/// `DefendAssignment` + `DirectMovementComponent` (center target
/// with [`DEFEND_IN_CELL_STOP_RADIUS`]) is inserted.
///
/// **Holding defenders** re-score every tick but only retarget when
/// another cell's score beats the current held cell's score by the
/// configured [`DEFEND_RETARGET_HYSTERESIS`] margin. Erased current
/// paint makes the held cell's score zero, so any remaining
/// candidate clears the margin and retargets immediately. This is
/// the "advance / redistribute" path: cross-cell spreading is
/// assignment-driven, not uncontrolled drift.
///
/// A defender in transit (`DefendAssignment` present) is not picked
/// up: it is already committed to its current move and will reach
/// the cell, enter hold, and only then be eligible for re-assignment
/// on a later tick. A defender en route to a charger or already
/// charging (`ChargerAssignment` / `ChargerProgress`) is also
/// skipped so the charge sustain loop owns it until it releases the
/// markers.
///
/// Per-iteration slot snapshots make same-tick picks spread: each
/// defender sees the cells earlier defenders in the same tick
/// already occupied, so a wave of idle defenders does not pile onto
/// the same closest cell.
#[allow(clippy::type_complexity)]
pub fn defender_assignment_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    mut slots: ResMut<SoftWorkSlots>,
    density: Res<CellDensity>,
    pressure: Res<DefendPressure>,
    defenders: Query<
        (
            Entity,
            &Transform,
            &Commitment,
            &NanobotType,
            &SwarmMember,
            Option<&DefendHold>,
        ),
        (
            With<Nanobot>,
            With<NanobotType>,
            Without<DefendAssignment>,
            Without<DirectMovementComponent>,
            // Defenders en route to a charger or already
            // charging must not be re-routed to a fresh
            // Defend cell until the charge loop releases
            // them. The rotation system drops the hold and
            // inserts the charger markers; the assignment
            // system must wait for both markers to clear.
            Without<ChargerAssignment>,
            Without<ChargerProgress>,
        ),
    >,
) {
    for (entity, transform, commitment, nanobot_type, swarm_member, hold) in &defenders {
        if *nanobot_type != NanobotType::Defender {
            continue;
        }
        if *commitment != Commitment::Idle {
            continue;
        }

        let defender_pos = transform.translation.truncate();
        let exclusion = DefendSelfExclusion {
            physical_cell: world_to_cell(defender_pos),
            reserved_cell: hold.map(|h| h.cell),
        };

        // Per-iteration snapshot so each defender sees the picks
        // made by earlier defenders in the same tick. Without
        // this, a swarm of defenders at the same starting point
        // would all pile onto the same closest Defend cell; with
        // it, soft work slot pressure spreads them across cells.
        let slots_snapshot = slots.clone();
        let Some(candidate) = best_defend_candidate(
            &grid,
            *commitment,
            defender_pos,
            &slots_snapshot,
            &density,
            &pressure,
            ZONE_BLOCK_SIZE,
            swarm_member.0,
            exclusion,
        ) else {
            continue;
        };

        // Holding defenders apply hysteresis before the shared
        // assignment path: keep the current cell unless another
        // cell beats it by the configured margin, and treat a
        // same-cell candidate as a no-op. Idle defenders skip
        // straight to the assignment.
        if let Some(old_hold) = hold {
            if candidate.cell == old_hold.cell {
                continue;
            }
            let current_score = score_specific_defend_cell(
                &grid,
                *commitment,
                defender_pos,
                old_hold.cell,
                &slots_snapshot,
                &density,
                &pressure,
                ZONE_BLOCK_SIZE,
                swarm_member.0,
                exclusion,
            )
            .unwrap_or(0.0);
            let threshold = current_score * (1.0 + DEFEND_RETARGET_HYSTERESIS);
            if candidate.score <= threshold {
                // No candidate clears hysteresis. The defender
                // keeps holding its current cell.
                continue;
            }
            // Retarget: release the old slot and drop the hold so
            // the shared assignment path below re-reserves the
            // new cell. Same-tick slot pressure on the new cell
            // prevents a second defender from piling on behind.
            slots.release(old_hold.cell, IntentKind::Defend);
            commands.entity(entity).remove::<DefendHold>();
        }

        // Shared assignment for the idle and retarget paths:
        // reserve the candidate cell against same-tick followers
        // and send the defender toward the cell center with the
        // in-cell stop radius so it counts as arrived once it is
        // meaningfully inside the cell.
        let cell_world = cell_center_world(candidate.cell);
        slots.occupy(candidate.cell, IntentKind::Defend);
        commands.entity(entity).insert((
            DefendAssignment {
                cell: candidate.cell,
            },
            DirectMovementComponent {
                xy: cell_world,
                stop_radius: DEFEND_IN_CELL_STOP_RADIUS,
            },
        ));
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
    defenders: Query<
        (Entity, &DefendAssignment),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<DefendAssignment>,
            Without<DirectMovementComponent>,
            Without<DefendHold>,
        ),
    >,
) {
    for (entity, assignment) in &defenders {
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
/// The slot is released only when the cell's paint is erased (the
/// defender returns to the assignment pool) or the assignment system
/// re-routes the defender to a new cell. The hold marker is removed
/// in both cases so the next tick's assignment pass sees an idle
/// defender.
#[allow(clippy::type_complexity)]
pub fn defender_hold_system(
    mut commands: Commands,
    mut slots: ResMut<SoftWorkSlots>,
    grid: Res<IntentGrid>,
    defenders: Query<
        (Entity, &DefendHold, &Transform, &NanobotType),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<DefendHold>,
            Without<DefendAssignment>,
        ),
    >,
) {
    for (entity, hold, transform, nanobot_type) in &defenders {
        if *nanobot_type != NanobotType::Defender {
            continue;
        }
        // If the Defend cell still has paint, the defender keeps
        // holding. The hold is "the cell is still painted", not
        // "the defender has been here for a while"; erasing the
        // paint releases the defender back to the assignment
        // pool.
        let still_painted = grid
            .cell(hold.cell)
            .is_some_and(|cell| cell.has(IntentKind::Defend));
        if !still_painted {
            slots.release(hold.cell, IntentKind::Defend);
            commands.entity(entity).remove::<DefendHold>();
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

/// Plugin that wires the defender systems into the Update schedule.
/// The chain runs after `move_velocity_system` so the movement
/// system has already pruned arrived bots (which is the trigger the
/// arrive system waits for). The density pass runs first so the
/// assignment scorer sees the post-movement physical layout; the
/// assignment system runs before arrive and hold so a freshly
/// repainted cell can re-route a holder on the same tick.
pub struct DefendPlugin;

impl Plugin for DefendPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CellDensity>();
        app.init_resource::<DefendPressure>();
        app.add_systems(
            Update,
            (
                cell_density_system,
                defender_assignment_system,
                defender_arrive_system,
                defender_hold_system,
            )
                .chain()
                .after(crate::nanobot::move_velocity_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests for `is_enemy_territory` and the
    //! spatial-pressure scoring model. The end-to-end contracts
    //! (defender selection, hold, advance, hysteresis) live in
    //! `tests/defend_zone_behavior.rs`.

    use super::*;
    use crate::intent::IntentGrid;

    #[test]
    fn cell_at_swarm_origin_is_friendly_territory() {
        // A cell at the Swarm's position is unambiguously
        // friendly: Chebyshev distance is 0, well within any
        // positive radius.
        let cell = IVec2::new(0, 0);
        let swarm = IVec2::new(0, 0);
        assert!(!is_enemy_territory(cell, swarm, 1));
        assert!(!is_enemy_territory(cell, swarm, 2));
    }

    #[test]
    fn cell_at_radius_boundary_is_friendly_territory() {
        // A cell whose Chebyshev distance equals the radius
        // sits exactly on the boundary and counts as
        // friendly. The check is `> radius`, not `>=`, so the
        // radius cell is friendly and the radius-plus-one cell
        // is enemy.
        let swarm = IVec2::new(0, 0);
        assert!(!is_enemy_territory(IVec2::new(1, 0), swarm, 1));
        assert!(!is_enemy_territory(IVec2::new(0, 1), swarm, 1));
        assert!(!is_enemy_territory(IVec2::new(1, 1), swarm, 1));
    }

    #[test]
    fn cell_one_past_radius_is_enemy_territory() {
        // Chebyshev distance strictly greater than the radius
        // is enemy territory. With radius 1, a cell two king-
        // moves away from the swarm is enemy.
        let swarm = IVec2::new(0, 0);
        assert!(is_enemy_territory(IVec2::new(2, 0), swarm, 1));
        assert!(is_enemy_territory(IVec2::new(0, 2), swarm, 1));
        assert!(is_enemy_territory(IVec2::new(2, 2), swarm, 1));
        assert!(is_enemy_territory(IVec2::new(-2, -1), swarm, 1));
    }

    #[test]
    fn territory_classification_is_symmetric_under_swarm_offset() {
        // Moving both the cell and the swarm by the same
        // vector must not change the classification. This pins
        // the "territory is a relative concept" contract.
        let cell = IVec2::new(5, -3);
        let swarm = IVec2::new(2, -1);
        let offset = IVec2::new(10, 7);
        let original = is_enemy_territory(cell, swarm, 1);
        let shifted = is_enemy_territory(cell + offset, swarm + offset, 1);
        assert_eq!(original, shifted);
    }

    #[test]
    fn zero_radius_makes_only_swarm_cell_friendly() {
        // A radius of 0 means only the Swarm's own cell is
        // friendly territory. Every other cell, including
        // diagonals, is enemy. This is the "no home
        // territory" edge case a follow-up issue can use.
        let swarm = IVec2::new(0, 0);
        assert!(!is_enemy_territory(IVec2::new(0, 0), swarm, 0));
        assert!(is_enemy_territory(IVec2::new(1, 0), swarm, 0));
        assert!(is_enemy_territory(IVec2::new(0, 1), swarm, 0));
        assert!(is_enemy_territory(IVec2::new(1, 1), swarm, 0));
    }

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

    #[test]
    fn best_defend_candidate_excludes_self_body_and_reservation() {
        // A holding defender re-scoring must not count its own
        // body or reservation, otherwise its own cell would look
        // crowded by itself. Two equally-painted, equidistant
        // cells with the defender holding one of them: the
        // held cell must score equal to (not below) the empty
        // cell, proving self-exclusion.
        let mut grid = IntentGrid::new(4, 4);
        let held = IVec2::new(-1, 0);
        let other = IVec2::new(1, 0);
        grid.paint(held, IntentKind::Defend, PAINT_STRENGTH_CAP);
        grid.paint(other, IntentKind::Defend, PAINT_STRENGTH_CAP);

        let mut slots = SoftWorkSlots::new();
        // The defender holds `held`: one reservation there.
        slots.occupy(held, IntentKind::Defend);
        let density = CellDensity::default();
        let pressure = DefendPressure::default();

        // Defender stands at the held cell center.
        let pos = cell_center_world_for_test(held);
        let exclusion = DefendSelfExclusion {
            physical_cell: held,
            reserved_cell: Some(held),
        };
        let candidate = best_defend_candidate(
            &grid,
            Commitment::Idle,
            pos,
            &slots,
            &density,
            &pressure,
            ZONE_BLOCK_SIZE,
            crate::nanobot::components::SwarmId::PLAYER,
            exclusion,
        )
        .expect("must find a candidate");

        // Without self-exclusion the held cell would be crowded
        // (1 body + 1 reservation) and lose to the empty `other`
        // cell. With self-exclusion both cells are equally
        // attractive, so the held cell (closer, distance ~0)
        // wins.
        assert_eq!(
            candidate.cell, held,
            "self-excluded held cell must beat or tie the empty cell"
        );
    }

    #[test]
    fn best_defend_candidate_prefers_higher_paint_for_occupancy() {
        // Stronger paint raises both attraction and capacity, so
        // at equal distance the strongly painted cell wins even
        // when it already has a reservation and the weak cell is
        // empty.
        let mut grid = IntentGrid::new(4, 4);
        let weak = IVec2::new(-1, 0);
        let strong = IVec2::new(1, 0);
        grid.paint(weak, IntentKind::Defend, 4);
        grid.paint(strong, IntentKind::Defend, PAINT_STRENGTH_CAP);

        let mut slots = SoftWorkSlots::new();
        // Pile a reservation on the strong cell so its occupancy
        // is 1; the weak cell stays empty (occupancy 0).
        slots.occupy(strong, IntentKind::Defend);
        let density = CellDensity::default();
        let pressure = DefendPressure::default();
        let pos = Vec2::new(0.0, 0.0);
        let exclusion = DefendSelfExclusion::default();

        let candidate = best_defend_candidate(
            &grid,
            Commitment::Idle,
            pos,
            &slots,
            &density,
            &pressure,
            ZONE_BLOCK_SIZE,
            crate::nanobot::components::SwarmId::PLAYER,
            exclusion,
        )
        .expect("must find a candidate");
        assert_eq!(
            candidate.cell, strong,
            "strong paint must beat weak paint despite one reservation"
        );
    }

    #[test]
    fn defend_pressure_hook_raises_a_cells_score() {
        // The pressure hook multiplies a cell's need. Two
        // equally-painted, equidistant cells: raising one cell's
        // pressure above baseline must make it win.
        let mut grid = IntentGrid::new(4, 4);
        let a = IVec2::new(-1, 0);
        let b = IVec2::new(1, 0);
        grid.paint(a, IntentKind::Defend, PAINT_STRENGTH_CAP);
        grid.paint(b, IntentKind::Defend, PAINT_STRENGTH_CAP);

        let slots = SoftWorkSlots::new();
        let density = CellDensity::default();
        let mut pressure = DefendPressure::default();
        pressure.set(b, 3.0);

        let pos = Vec2::new(0.0, 0.0);
        let exclusion = DefendSelfExclusion::default();
        let candidate = best_defend_candidate(
            &grid,
            Commitment::Idle,
            pos,
            &slots,
            &density,
            &pressure,
            ZONE_BLOCK_SIZE,
            crate::nanobot::components::SwarmId::PLAYER,
            exclusion,
        )
        .expect("must find a candidate");
        assert_eq!(candidate.cell, b, "pressurised cell must win");
    }

    #[test]
    fn best_defend_candidate_returns_none_when_no_defend_paint() {
        let grid = IntentGrid::new(4, 4);
        let slots = SoftWorkSlots::new();
        let density = CellDensity::default();
        let pressure = DefendPressure::default();
        let candidate = best_defend_candidate(
            &grid,
            Commitment::Idle,
            Vec2::new(0.0, 0.0),
            &slots,
            &density,
            &pressure,
            ZONE_BLOCK_SIZE,
            crate::nanobot::components::SwarmId::PLAYER,
            DefendSelfExclusion::default(),
        );
        assert!(candidate.is_none());
    }

    /// Local copy of the cell-center formula so this test module
    /// does not depend on the private `cell_center_world` helper.
    fn cell_center_world_for_test(cell: IVec2) -> Vec2 {
        Vec2::new(
            (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
            (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
        )
    }
}
