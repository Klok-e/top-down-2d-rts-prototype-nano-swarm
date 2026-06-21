//! Defend Zone behavior for Defender nanobots.
//!
//! Issue #13 contract: Defenders protect assets in Defend Zones and
//! advance when Defend Zone intent is painted into enemy territory.
//! This is the initial attack/advance behavior; no separate Attack
//! Zone is added. Combat uses swarm systems rather than group
//! commands -- every Defender is an autonomous agent that picks its
//! own work from the global Defend intent.
//!
//! State machine carried on the defender by marker components:
//!
//! ```text
//!   Idle -> (assignment system) -> Moving (DefendAssignment + DMC)
//!   Moving -> (arrive system)   -> Holding (DefendAssignment + DefendHold)
//!   Holding -> (assignment system) -> Moving (new DefendAssignment)
//! ```
//!
//! "Enemy territory" is defined as a Defend cell whose Chebyshev
//! distance from the Swarm's cell is greater than
//! [`DEFEND_HOME_RADIUS_CELLS`]. Cells inside the radius are
//! "friendly territory" and defenders holding there are guarding the
//! swarm; cells outside are the frontier the swarm is pushing into.
//! Painting Defend intent in enemy territory is what triggers the
//! "advance" behavior: a defender at the frontier re-scores and walks
//! outward.
//!
//! The hold position itself is the cell's world center. A defending
//! defender stands in the cell and does not move until the assignment
//! system re-routes them. The soft work slot stays occupied while
//! they hold so a wave of defenders does not pile onto the same cell.

use bevy::prelude::*;

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{best_candidate, Commitment, NanobotType, SoftWorkSlots};
use crate::nanobot::charge::{ChargerAssignment, ChargerProgress};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, SwarmMember};
use crate::nanobot::consts::STOP_THRESHOLD;
use crate::ZONE_BLOCK_SIZE;

/// Number of cells around the Swarm that count as "friendly
/// territory". A Defend cell at Chebyshev distance greater than this
/// from the Swarm's cell is "enemy territory" and triggers the
/// advance behavior. Tuned to be small enough that the test grid
/// (8x8) has both friendly and enemy cells relative to a Swarm at
/// (0, 0), and large enough that a single Defend cell painted at the
/// Swarm origin is unambiguously friendly.
pub const DEFEND_HOME_RADIUS_CELLS: i32 = 1;

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

/// Marks a Defender as committed to a specific Defend cell. Set by
/// the assignment system when the defender picks a Defend candidate;
/// cleared when the defender transitions into hold state (the
/// `DefendHold` marker takes over) or when the defender is re-routed
/// to a new cell.
#[derive(Debug, Component, Clone, Copy)]
pub struct DefendAssignment {
    pub cell: IVec2,
}

/// Marks a Defender that has arrived at its assigned Defend cell and
/// is now "holding" the position. The defender has no
/// [`DirectMovementComponent`] while holding -- the move system has
/// pruned it on arrival, which is the trigger for inserting this
/// marker. The soft work slot is occupied by the assignment system
/// before the defender arrives, so the slot is still considered
/// "in use" while the defender holds; the hold system releases it
/// when the defender leaves the cell.
#[derive(Debug, Component, Clone, Copy)]
pub struct DefendHold {
    pub cell: IVec2,
}

/// For each Defender that is idle OR holding a cell, score the
/// Defend intent globally and (re)assign the defender to the
/// best-scoring cell. The (cell, Defend) soft work slot is occupied
/// for the new cell, and the old cell's slot (if any) is released.
///
/// A holding defender is re-routable so a fresh Defend cell painted
/// further from the swarm can pull defenders out of an interior
/// hold position. This is the "advance into enemy territory"
/// behavior: the assignment scorer picks the new cell (a defender
/// at the frontier has a closer or higher-painted candidate), the
/// hold is released, the defender walks outward.
///
/// A defender that is in transit (`DefendAssignment` present) is
/// not picked up: it is already committed to its current move and
/// will reach the cell, enter hold, and only then be eligible for
/// re-assignment on a later tick.
///
/// The scoring already routes Defenders to Defend intent (type fit =
/// 1.0) and rejects Defenders from Gather/Build/Corridor (type fit =
/// 0), so the assignment system just has to follow
/// `best_candidate` and trust the global scoring. The Defend-only
/// kinds filter is what wires "Defenders choose Defend Zone work
/// from autonomy scoring" in the acceptance criteria.
#[allow(clippy::type_complexity)]
pub fn defender_assignment_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    mut slots: ResMut<SoftWorkSlots>,
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
        // Per-iteration snapshot so each defender sees the picks
        // made by earlier defenders in the same tick. Without
        // this, a swarm of defenders at the same starting point
        // would all pick the same closest Defend cell; with it,
        // soft work slot pressure spreads them across distinct
        // cells. The snapshot is cheap because SoftWorkSlots is
        // a small HashMap.
        let slots_snapshot = slots.clone();
        let Some(candidate) = best_candidate(
            &grid,
            NanobotType::Defender,
            *commitment,
            defender_pos,
            &slots_snapshot,
            ZONE_BLOCK_SIZE,
            &[IntentKind::Defend],
            swarm_member.0,
        ) else {
            continue;
        };
        if candidate.kind != IntentKind::Defend {
            // Belt-and-braces: best_candidate only returns
            // candidates for kinds it is asked to score, but the
            // explicit check keeps the contract clear.
            continue;
        }

        // Release the old hold's slot before occupying the new
        // one. Without this, a defender re-routed from cell A to
        // cell B would leave the (A, Defend) slot stuck at 1 and
        // a fresh defender would see A as already busy.
        if let Some(old_hold) = hold {
            if old_hold.cell != candidate.cell {
                slots.release(old_hold.cell, IntentKind::Defend);
                commands.entity(entity).remove::<DefendHold>();
            } else {
                // Same cell as the new pick -- no work to do, no
                // re-route. The defender keeps holding.
                continue;
            }
        }

        let cell_world = Vec2::new(
            (candidate.cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
            (candidate.cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
        );
        slots.occupy(candidate.cell, IntentKind::Defend);
        commands.entity(entity).insert((
            DefendAssignment {
                cell: candidate.cell,
            },
            DirectMovementComponent { xy: cell_world },
        ));
    }
}

/// Detect a Defender that has arrived at its assigned Defend cell
/// and transition it into the hold state. The trigger is the same
/// as the rest of the simulation: the movement system removes the
/// [`DirectMovementComponent`] when the defender is within
/// [`STOP_THRESHOLD`] of its destination.
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

/// Keep Defenders that have arrived at their Defend cell in place.
/// The arrival system inserts `DefendHold` and removes the
/// `DefendAssignment`; this system runs after the assignment
/// system so a defender whose Defend cell is repainted (or
/// erased) can be picked up by the assignment system on the next
/// tick and routed to a new cell.
///
/// The hold system does not move the defender -- a `DefendHold`
/// defender has no `DirectMovementComponent` unless they drifted
/// off-center due to separation forces, in which case the system
/// re-snaps the DMC to the cell center. The slot is released only
/// when the defender transitions out of hold state (i.e. the
/// assignment system inserts a new `DefendAssignment` for a new
/// cell, or the cell's paint is erased), so the slot is "in use"
/// for the entire hold duration.
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
        // Pin the defender's world position to the cell center
        // so a defender who drifted due to separation forces
        // does not slowly walk off the cell. The hold contract
        // is "stay in the cell", and the cell is a square zone
        // -- snap to the center.
        let cell_center = Vec2::new(
            (hold.cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
            (hold.cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
        );
        let current = transform.translation.truncate();
        if current.distance(cell_center) > STOP_THRESHOLD {
            commands
                .entity(entity)
                .insert(DirectMovementComponent { xy: cell_center });
        }
    }
}

/// Plugin that wires the defender systems into the Update schedule.
/// The chain runs after `move_velocity_system` so the movement
/// system has already pruned arrived bots (which is the trigger the
/// arrive system waits for). The hold system runs after the
/// assignment system so a defender that has been holding for a
/// while and whose Defend cell has been repainted gets re-routed
/// on the next tick.
pub struct DefendPlugin;

impl Plugin for DefendPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
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
    //! Pure-helper unit tests for `is_enemy_territory`. The
    //! end-to-end contracts (defender selection, hold, advance)
    //! live in `tests/defend_zone_behavior.rs`.

    use super::*;

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
}
