//! Structure maintenance and degradation for issue #12.
//!
//! Structures require ongoing worker time to remain functional.
//! The maintenance layer is a "leaky bucket" model:
//!
//! ```text
//!   Structure.ticks_since_maintained
//!     -> increments every tick
//!     -> reset to 0 each tick a worker spends maintaining
//!     -> past the buffer, the structure loses 1 health per tick
//!     -> at 0 health, the structure collapses and is despawned
//! ```
//!
//! Maintenance consumes only worker time. The maintenance work
//! system never reads a stockpile, never pulls from the resource
//! ledger, and never advances a build site. A worker assigned to
//! maintenance spends `MAINTENANCE_WORK_DURATION_TICKS` ticks
//! holding the structure's `ticks_since_maintained` at 0 and
//! restoring a small amount of health per tick, then is freed.
//!
//! Worker state machine for the maintenance path:
//!
//! ```text
//!   Idle -> (assignment system) -> Moving (MaintenanceAssignment + DMC)
//!   Moving -> (arrive system)   -> Working (MaintenanceProgress)
//!   Working -> (work system)    -> Working (no resource, just time)
//!   Working -> (work duration reached OR target collapsed)
//!            -> Idle
//! ```
//!
//! The build layer (issue #10) is the demand signal: a worker
//! scoring a Build cell first looks for a build site, then a
//! damaged structure (repair), and now also looks for a structure
//! that needs maintenance. The maintenance work is the cheapest
//! of the three (no material, no travel budget pressure) so the
//! build cell's soft work slot is reused for it. A future
//! tracking issue can split maintenance into its own slot if
//! player feedback shows crowding.

use bevy::prelude::*;

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{Commitment, NanobotType, SoftWorkSlots, best_candidate};
use crate::nanobot::components::SwarmMember;
// `StructureKind` is only used by the unit tests in this
// module. Marked `allow(unused_imports)` so the lib build does
// not warn; the tests do pick the import up via `use super::*`.
use crate::ZONE_BLOCK_SIZE;
#[allow(unused_imports)]
use crate::nanobot::build::{Structure, StructureKind};
use crate::nanobot::components::{DirectMovementComponent, Nanobot};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::placement::BUILDING_FOOTPRINT_RADIUS;

/// How many ticks a structure stays stable after a maintenance
/// shift. The buffer gives the swarm room to come back later
/// without the structure immediately starting to lose health.
/// Tuned so a single worker can cycle through a handful of
/// structures without any of them degrading.
pub const MAINTENANCE_BUFFER_TICKS: u32 = 50;

/// Number of ticks a worker spends on a single maintenance
/// shift. The worker is "permanently" allocated to maintenance
/// for this many ticks, then released back to the autonomy
/// scorer. A short shift keeps the worker responsive to other
/// work (gather, build) but long enough that the structure
/// actually receives meaningful work.
pub const MAINTENANCE_WORK_DURATION_TICKS: u32 = 5;

/// Health restored per tick of maintenance work. Combined with
/// `MAINTENANCE_WORK_DURATION_TICKS`, a single shift restores
/// 10 health -- enough to fully repair a lightly degraded
/// structure and to keep a fully-degraded one trending upward
/// under repeated visits.
pub const MAINTENANCE_HEALTH_PER_TICK: u32 = 2;

/// Health lost per tick once the buffer has expired. One per
/// tick keeps the math obvious and matches the per-tick
/// resource-granularity used by the rest of the simulation.
pub const DEGRADATION_PER_TICK: u32 = 1;

/// A structure is considered "needing maintenance" once its
/// buffer counter reaches this value. The threshold sits well
/// below `MAINTENANCE_BUFFER_TICKS` so a worker can finish a
/// shift and return before the structure actually starts
/// losing health. A damaged structure (`health < max`) is also
/// "needy" so combat damage pulls workers back.
pub const MAINTENANCE_NEEDS_THRESHOLD: u32 = 25;

impl Structure {
    /// True when the structure is a valid maintenance target.
    /// Either the buffer is expired (`ticks_since_maintained`
    /// has reached the threshold) or the structure has been
    /// damaged and is below max health. The maintenance
    /// assignment system uses this to filter the world query.
    pub fn needs_maintenance(&self) -> bool {
        self.ticks_since_maintained >= MAINTENANCE_NEEDS_THRESHOLD
            || self.health < super::build::STRUCTURE_MAX_HEALTH
    }
}

/// Marks a Worker as committed to maintaining a specific
/// structure in a specific cell. `target` is the `Structure`
/// entity the worker is travelling to. Set by the assignment
/// system, cleared when the worker arrives and starts work (or
/// when the target collapses or disappears).
#[derive(Debug, Component, Clone, Copy)]
pub struct MaintenanceAssignment {
    pub cell: IVec2,
    pub target: Entity,
}

/// In-flight maintenance work. The worker is at the target and
/// is doing the per-tick maintenance action (no resource, just
/// time). `ticks_worked` tracks how many of
/// `MAINTENANCE_WORK_DURATION_TICKS` the worker has spent; the
/// work system releases the worker once the budget is reached.
#[derive(Debug, Component, Clone, Copy)]
pub struct MaintenanceProgress {
    pub cell: IVec2,
    pub target: Entity,
    pub ticks_worked: u32,
}

/// Increment `ticks_since_maintained` for every structure every
/// tick, then degrade the structure once the buffer has been
/// exceeded. A structure that has been freshly maintained sees
/// its counter reset by the work system before this system
/// runs, so the order of systems matters: the work system must
/// precede the degradation system within the same chain so the
/// "maintenance was just done" tick is not also a "degrade one
/// health" tick. The `MaintenancePlugin` wires the order.
///
/// Structures that have just collapsed (health reached zero on
/// the previous tick) are despawned here so the rest of the
/// chain does not see a zero-health structure lingering in
/// queries. The despawn is a hard remove: there is no
/// `Collapsed` marker, no debris, no recoverable corpse in the
/// first implementation. A future issue can add a collapse
/// animation or recovery flow without changing the public
/// contract.
#[allow(clippy::type_complexity)]
pub fn structure_degradation_system(
    mut commands: Commands,
    mut structures: Query<(Entity, &mut Structure)>,
) {
    for (entity, mut structure) in &mut structures {
        // Always advance the buffer counter. Workers reset it
        // to 0 in the same tick; everything else sees it grow.
        structure.ticks_since_maintained = structure.ticks_since_maintained.saturating_add(1);

        if structure.ticks_since_maintained > MAINTENANCE_BUFFER_TICKS {
            // Buffer expired; the structure is unstable and
            // starts losing health this tick.
            let next_health = structure.health.saturating_sub(DEGRADATION_PER_TICK);
            structure.health = next_health;
            if next_health == 0 {
                // Collapse: remove the structure from the
                // world. The cell becomes a valid build site
                // again because auto-creation skips cells
                // that already hold a Structure.
                commands.entity(entity).despawn();
            }
        }
    }
}

/// For each idle Worker with no in-flight build, repair, or
/// maintenance work, pick a Build cell through the autonomy
/// scorer and, if the cell contains a structure that needs
/// maintenance, assign the worker to the nearest such
/// structure. The (cell, Build) soft work slot is occupied so
/// future assignees see the cell as busier.
///
/// The same Build soft work slot is reused for maintenance
/// because the two are closely related: a build cell with no
/// construction work is the natural place for upkeep. Workers
/// doing build or repair in the same cell already release
/// their slot, so the maintenance assignment can pick the cell
/// back up. The capacity is shared, not duplicated.
#[allow(clippy::type_complexity)]
pub fn worker_maintenance_assignment_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    mut slots: ResMut<SoftWorkSlots>,
    workers: Query<
        (Entity, &Transform, &Commitment, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            Without<super::build::BuildAssignment>,
            Without<super::build::BuildProgress>,
            Without<MaintenanceAssignment>,
            Without<MaintenanceProgress>,
            Without<DirectMovementComponent>,
        ),
    >,
    structures: Query<(Entity, &Transform, &Structure)>,
) {
    let slots_snapshot = slots.clone();
    for (entity, transform, commitment, nanobot_type, swarm_member) in &workers {
        if *nanobot_type != NanobotType::Worker {
            continue;
        }
        if *commitment != Commitment::Idle {
            continue;
        }

        let worker_pos = transform.translation.truncate();
        let Some(candidate) = best_candidate(
            &grid,
            NanobotType::Worker,
            *commitment,
            worker_pos,
            &slots_snapshot,
            ZONE_BLOCK_SIZE,
            &[IntentKind::Build],
            swarm_member.0,
        ) else {
            continue;
        };
        if candidate.kind != IntentKind::Build {
            continue;
        }

        // Find the nearest structure in the chosen cell that
        // is asking for maintenance. If nothing in the cell
        // needs maintenance, the worker stays idle -- the
        // build assignment system will run next tick and may
        // pick a build site or a damaged structure.
        let Some((target_entity, target_pos)) =
            find_nearest_needy_structure(candidate.cell, worker_pos, &structures)
        else {
            continue;
        };

        slots.occupy(candidate.cell, IntentKind::Build);
        commands.entity(entity).insert((
            MaintenanceAssignment {
                cell: candidate.cell,
                target: target_entity,
            },
            // Issue #38 / ADR-0004: stop on the
            // building footprint's physical edge
            // so the worker lands at the
            // structure's centre, matching the
            // arrive guard.
            DirectMovementComponent {
                xy: target_pos,
                stop_radius: BUILDING_FOOTPRINT_RADIUS,
            },
        ));
    }
}

fn find_nearest_needy_structure(
    cell: IVec2,
    worker_pos: Vec2,
    structures: &Query<(Entity, &Transform, &Structure)>,
) -> Option<(Entity, Vec2)> {
    let mut best: Option<(f32, Entity, Vec2)> = None;
    for (entity, transform, structure) in structures.iter() {
        if world_to_cell(transform.translation.truncate()) != cell {
            continue;
        }
        if !structure.needs_maintenance() {
            continue;
        }
        let pos = transform.translation.truncate();
        let d = worker_pos.distance(pos);
        if best.is_none_or(|(bd, _, _)| d < bd) {
            best = Some((d, entity, pos));
        }
    }
    best.map(|(_, e, pos)| (e, pos))
}

/// Detect a worker that has arrived at its assigned maintenance
/// target and start the work phase. The `Without<MaintenanceProgress>`
/// filter makes arrival idempotent: the same tick cannot fire
/// twice. The trigger is the same as the build chain: the
/// movement system removes `DirectMovementComponent` when the
/// bot is close enough to its target. The arrival threshold
/// matches the building footprint (issue #38 / ADR-0004) so
/// the worker lands at the structure's centre, not at
/// `centre + STOP_THRESHOLD`. The guard mirrors the
/// `DirectMovementComponent::stop_radius` the assignment
/// system passes.
#[allow(clippy::type_complexity)]
pub fn worker_maintenance_arrive_system(
    mut commands: Commands,
    workers: Query<
        (Entity, &Transform, &MaintenanceAssignment),
        (
            With<Nanobot>,
            With<MaintenanceAssignment>,
            Without<DirectMovementComponent>,
            Without<MaintenanceProgress>,
        ),
    >,
    structures: Query<&Transform, With<Structure>>,
) {
    for (entity, transform, assignment) in &workers {
        let Ok(target_transform) = structures.get(assignment.target) else {
            // Target collapsed between assignment and arrival,
            // or was otherwise removed. Release the slot and
            // drop the assignment; the worker idles.
            commands.entity(entity).remove::<MaintenanceAssignment>();
            continue;
        };
        let distance = transform
            .translation
            .truncate()
            .distance(target_transform.translation.truncate());
        if distance <= BUILDING_FOOTPRINT_RADIUS {
            commands.entity(entity).insert(MaintenanceProgress {
                cell: assignment.cell,
                target: assignment.target,
                ticks_worked: 0,
            });
        } else {
            // Resume branch (issue #38 / ADR-0004):
            // the `Without<DirectMovementComponent>`
            // filter guarantees the worker has no
            // DMC, so re-issue one with the same
            // extent the assignment path uses. A
            // bot nudged past the footprint by
            // separation force walks back instead
            // of stalling without a movement
            // command.
            commands.entity(entity).insert(DirectMovementComponent {
                xy: target_transform.translation.truncate(),
                stop_radius: BUILDING_FOOTPRINT_RADIUS,
            });
        }
    }
}

/// Worker maintenance work system. For each worker with a
/// [`MaintenanceProgress`], reset the target structure's
/// buffer counter to 0 and restore a small amount of health
/// (capped at the max). Increment the worker's
/// `ticks_worked`; when the budget is reached, release the
/// worker back to idle.
///
/// The system does **not** read a stockpile, does **not** touch
/// the resource ledger, and does **not** create a build site.
/// That is the "consumes Worker time only, not extra resources"
/// contract from issue #12. A worker doing maintenance
/// produces zero resource churn.
///
/// The system must run **before** [`structure_degradation_system`]
/// within the same tick so the freshly-reset buffer counter is
/// not incremented in the same tick -- the worker counts as
/// having maintained the structure "this tick" and the
/// structure is not also "one tick closer to degrading".
#[allow(clippy::type_complexity)]
pub fn worker_maintenance_work_system(
    mut commands: Commands,
    mut workers: Query<(Entity, &mut MaintenanceProgress), With<Nanobot>>,
    mut structures: Query<&mut Structure>,
) {
    for (entity, mut progress) in &mut workers {
        let Ok(mut structure) = structures.get_mut(progress.target) else {
            // Target collapsed between arrival and work (e.g.
            // a previous test or a future system despawned it).
            // Release the worker; the cell becomes a valid
            // build site again on the next auto-creation tick.
            release_maintenance_worker(&mut commands, entity);
            continue;
        };

        // Reset the buffer counter to 0 BEFORE the
        // degradation system runs. The worker's "I just
        // maintained this" stamp is the reset.
        structure.ticks_since_maintained = 0;
        structure.health = (structure.health + MAINTENANCE_HEALTH_PER_TICK)
            .min(super::build::STRUCTURE_MAX_HEALTH);

        progress.ticks_worked += 1;
        if progress.ticks_worked >= MAINTENANCE_WORK_DURATION_TICKS {
            release_maintenance_worker(&mut commands, entity);
        }
    }
}

/// Clear maintenance lifecycle markers.
fn release_maintenance_worker(commands: &mut Commands, entity: Entity) {
    commands
        .entity(entity)
        .remove::<MaintenanceAssignment>()
        .remove::<MaintenanceProgress>();
}

/// Plugin that wires the maintenance systems into the Update
/// schedule. The chain runs after `move_velocity_system` so
/// the movement system has already pruned arrived bots, and
/// the maintenance work system runs before the degradation
/// system so a fresh "I just maintained this" stamp is not
/// also a "degrade one health" tick. The chain also runs
/// after the build work system so the build layer gets first
/// pick on workers and maintenance only catches the ones
/// build did not need.
pub struct MaintenancePlugin;

impl Plugin for MaintenancePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                worker_maintenance_arrive_system,
                worker_maintenance_work_system,
                structure_degradation_system,
            )
                .chain()
                .after(crate::nanobot::RegionalAllocationSet::Acquire)
                .after(crate::nanobot::move_velocity_system)
                .after(super::build::worker_build_work_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests for the maintenance data and
    //! constants. The end-to-end contracts (degradation,
    //! collapse, stable maintenance) live in
    //! `tests/maintenance_behavior.rs`.

    use super::*;

    #[test]
    fn maintenance_buffer_is_longer_than_work_duration() {
        // A worker must be able to finish a maintenance shift
        // and find something else to do before the structure
        // starts losing health. The buffer needs to be larger
        // than the work duration for the "stable under enough
        // worker time" contract to make sense.
        const { assert!(MAINTENANCE_BUFFER_TICKS > MAINTENANCE_WORK_DURATION_TICKS) };
    }

    #[test]
    fn maintenance_needs_threshold_sits_inside_buffer() {
        // The needs-threshold is the early-warning line that
        // pulls workers back before degradation starts. It
        // must sit strictly between 0 and the buffer so a
        // freshly-maintained structure is not immediately
        // flagged as needing maintenance again.
        const {
            assert!(MAINTENANCE_NEEDS_THRESHOLD > 0);
            assert!(MAINTENANCE_NEEDS_THRESHOLD < MAINTENANCE_BUFFER_TICKS);
        };
    }

    #[test]
    fn maintenance_shift_recovers_at_least_one_degradation_step() {
        // A single maintenance shift must restore at least
        // one tick's worth of degradation damage. Otherwise
        // a worker maintaining once cannot keep up with a
        // structure that is already losing health.
        const { assert!(MAINTENANCE_HEALTH_PER_TICK >= DEGRADATION_PER_TICK) };
    }

    #[test]
    fn structure_at_full_health_and_fresh_does_not_need_maintenance_yet() {
        // A freshly-built structure is at full health with a
        // zeroed buffer counter. It must not be flagged as
        // needing maintenance until the threshold is reached.
        let s = Structure::new(StructureKind::Basic);
        assert!(s.is_full_health());
        assert_eq!(s.ticks_since_maintained, 0);
        assert!(
            !s.needs_maintenance(),
            "fresh structure must not be a maintenance target"
        );
    }

    #[test]
    fn structure_needs_maintenance_once_buffer_approaches() {
        // Pin the "approaching the buffer" branch of
        // `needs_maintenance`. The threshold is the line that
        // pulls a worker back early.
        let mut s = Structure::new(StructureKind::Basic);
        s.ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD - 1;
        assert!(
            !s.needs_maintenance(),
            "structure just below the threshold is still stable"
        );
        s.ticks_since_maintained = MAINTENANCE_NEEDS_THRESHOLD;
        assert!(
            s.needs_maintenance(),
            "structure at the threshold needs maintenance"
        );
    }

    #[test]
    fn damaged_structure_needs_maintenance_even_if_fresh() {
        // A structure with health below the max is a valid
        // maintenance target regardless of the buffer
        // counter. Combat damage or repair shortfalls pull
        // workers back even if the buffer has not yet
        // expired.
        let mut s = Structure::new(StructureKind::Basic);
        s.health = super::super::build::STRUCTURE_MAX_HEALTH - 1;
        s.ticks_since_maintained = 0;
        assert!(s.needs_maintenance());
    }

    #[test]
    fn collapsed_structure_is_detected() {
        let mut s = Structure::new(StructureKind::Basic);
        assert!(!s.is_collapsed());
        s.health = 1;
        assert!(!s.is_collapsed());
        s.health = 0;
        assert!(s.is_collapsed());
    }
}
