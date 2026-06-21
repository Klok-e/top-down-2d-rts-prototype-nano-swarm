//! Build Zone behavior for Worker nanobots.
//!
//! Issue #10 contract: Build Zones cause Workers to construct or
//! repair player structures where sustained work is needed. Build
//! Zones use local stockpile-backed material flow, and automatic
//! construction happens inside or near matching intent paint.
//!
//! State machine carried on the worker by marker components:
//!
//! ```text
//!   Idle -> (assignment system) -> Moving (BuildAssignment + DMC)
//!   Moving -> (arrive system) -> Working (BuildProgress)
//!   Working -> (work system) -> Working (consume material each tick)
//!   Working -> (no material left OR site complete OR repair done)
//!            -> Idle
//! ```
//!
//! Build sites auto-emerge inside Build-painted cells with no
//! existing structure. The first implementation has a single
//! structure kind (`StructureKind::Basic`); later issues can add
//! production facilities, chargers, and other support structures.
//!
//! Material flow: the worker at a build site pulls one unit of
//! `ResourceKind::Minerals` from the nearest local stockpile per
//! `app.update()` tick. The hauler chain (issue #8) delivers
//! material to those stockpiles, so the worker is consuming
//! resources that were already moved physically. No teleporting
//! resources; no global stockpile.
//!
//! Repair: a completed `Structure` with `health < STRUCTURE_MAX_HEALTH`
//! is also valid build work. A worker assigned to a Build cell
//! that already holds a damaged Structure (and no BuildSite) will
//! be assigned to the Structure, then consume material to restore
//! health up to the cap.

use bevy::prelude::*;

use crate::ai::get_world_from_zone;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{best_candidate, Commitment, NanobotType, SoftWorkSlots};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, SwarmMember};
use crate::nanobot::consts::STOP_THRESHOLD;
use crate::nanobot::gather::world_to_cell;
use crate::resources::{ResourceKind, ResourceLedger, Stockpile};
use crate::ZONE_BLOCK_SIZE;

/// Maximum health a `Structure` can have. Repair and construction
/// stop raising health at this cap. A structure starts at full
/// health and can degrade (e.g. via the maintenance system from
/// issue #12, modelled as a manual health drop in tests for now).
pub const STRUCTURE_MAX_HEALTH: u32 = 100;

/// Material budget required to construct a fresh `BuildSite`.
///
/// Each tick a working worker consumes one unit of
/// `ResourceKind::Minerals` from the nearest local stockpile and
/// the site's `consumed_materials` rises by one. When the counter
/// reaches the budget, the site is replaced by a completed
/// `Structure`. The number is small enough that a single worker
/// finishes the site in a handful of ticks, keeping the
/// end-to-end tests fast.
pub const BUILD_REQUIRED_MATERIALS: u32 = 10;

/// Health restored per unit of material consumed during repair.
/// Matches `BUILD_REQUIRED_MATERIALS * BUILD_HEALTH_PER_MATERIAL`
/// == `STRUCTURE_MAX_HEALTH` so a fully degraded structure can be
/// repaired with the same material budget as a fresh build.
pub const BUILD_HEALTH_PER_MATERIAL: u32 = 10;

/// Distinct kinds of structures the swarm can build. The first
/// implementation only models `Basic`; later issues (production
/// facilities, chargers) extend this enum.
#[derive(Debug, Component, Default, Clone, Copy, PartialEq, Eq)]
pub enum StructureKind {
    #[default]
    Basic,
}

/// A completed structure in the world. `health` is in
/// `[0, STRUCTURE_MAX_HEALTH]`. A new structure starts at full
/// health; degradation (issue #12) lowers it. The build work
/// system treats a `Structure` with `health < max` as repair work
/// and raises the value back to the cap.
///
/// `ticks_since_maintained` is the maintenance buffer counter
/// from issue #12. The maintenance system resets it to 0 when a
/// worker maintains the structure, and increments it every tick
/// otherwise. When the counter exceeds the buffer, the structure
/// starts losing health. The counter lives on the structure so
/// the maintenance work system can reset it without searching
/// for a separate state object.
#[derive(Debug, Component, Default, Clone, Copy)]
pub struct Structure {
    pub kind: StructureKind,
    pub health: u32,
    pub ticks_since_maintained: u32,
}

impl Structure {
    /// Build a new structure at full health. Used by tests and
    /// by the completion system when a build site finishes.
    pub fn new(kind: StructureKind) -> Self {
        Self {
            kind,
            health: STRUCTURE_MAX_HEALTH,
            ticks_since_maintained: 0,
        }
    }

    /// True when the structure is at or above max health. Used
    /// by the build work system to decide whether repair is
    /// still useful.
    pub fn is_full_health(&self) -> bool {
        self.health >= STRUCTURE_MAX_HEALTH
    }

    /// True when the structure has just collapsed (no health
    /// left). The degradation system uses this to despawn
    /// collapsed structures. A structure that has not been
    /// collapsed has at least 1 health point.
    pub fn is_collapsed(&self) -> bool {
        self.health == 0
    }
}

/// A `BuildSite` is a `Structure` that is still under
/// construction. The site tracks how much material has been
/// consumed; when `consumed_materials >= required_materials` the
/// completion system removes the `BuildSite` and inserts a
/// `Structure` at the same position.
#[derive(Debug, Component, Clone, Copy)]
pub struct BuildSite {
    pub cell: IVec2,
    pub kind: StructureKind,
    pub required_materials: u32,
    pub consumed_materials: u32,
}

impl BuildSite {
    /// Spawn a new `BuildSite` in `cell` with the default
    /// material budget. `consumed_materials` starts at 0.
    pub fn new(cell: IVec2, kind: StructureKind) -> Self {
        Self {
            cell,
            kind,
            required_materials: BUILD_REQUIRED_MATERIALS,
            consumed_materials: 0,
        }
    }

    /// True when the site has consumed its full material budget
    /// and is ready to be promoted to a `Structure`.
    pub fn is_complete(&self) -> bool {
        self.consumed_materials >= self.required_materials
    }
}

/// Marks a Worker as committed to a specific build target in a
/// specific cell. `target` is either a `BuildSite` entity (under
/// construction) or a `Structure` entity (awaiting repair). The
/// assignment is set by the build assignment system and cleared
/// when the work finishes.
#[derive(Debug, Component, Clone, Copy)]
pub struct BuildAssignment {
    pub cell: IVec2,
    pub target: Entity,
}

/// In-flight build work. The worker is at the target and is
/// consuming material each tick. The component carries the same
/// `cell` and `target` as `BuildAssignment` so the work system
/// does not need to look up the original assignment to find what
/// the worker is working on.
#[derive(Debug, Component, Clone, Copy)]
pub struct BuildProgress {
    pub cell: IVec2,
    pub target: Entity,
}

/// Walk the [`IntentGrid`] and spawn a new [`BuildSite`] in any
/// Build cell that has paint but no existing structure or
/// build site. The acceptance criterion is "automatic support
/// construction happens inside or near matching intent paint":
/// the player's paint is the demand signal, the swarm provides
/// the construction target.
///
/// Cells that already hold a `BuildSite` or a `Structure` are
/// skipped so the swarm cannot pile multiple construction
/// targets into a single cell. The site is spawned at the
/// cell's world center so workers can walk straight to it.
#[allow(clippy::type_complexity)]
pub fn structure_auto_creation_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    existing_sites: Query<&Transform, Or<(With<BuildSite>, With<Structure>)>>,
) {
    let mut cells_with_target: std::collections::HashSet<IVec2> = std::collections::HashSet::new();
    for transform in &existing_sites {
        cells_with_target.insert(world_to_cell(transform.translation.truncate()));
    }
    for (cell, intent_cell) in grid.iter_cells() {
        if !intent_cell.has(IntentKind::Build) {
            continue;
        }
        if cells_with_target.contains(&cell) {
            continue;
        }
        commands.spawn((
            BuildSite::new(cell, StructureKind::Basic),
            Transform::from_translation(get_world_from_zone(cell).extend(0.0)),
        ));
    }
}

/// For each idle Worker with no in-flight build work, pick a
/// Build cell through the autonomy scoring from issue #6 and
/// assign the worker to the nearest build target in that cell.
/// The (cell, Build) soft work slot is occupied so future
/// assignees see the cell as busier.
///
/// Build targets in priority order:
///   1. A `BuildSite` in the cell (under construction).
///   2. A `Structure` in the cell with `health < max` (needs
///      repair). Falls back to the first damaged structure in
///      the cell so the scoring does not need to rank repairs
///      against fresh builds.
///
/// If the cell has no build target, the worker stays idle. The
/// `BuildSite` auto-creation system will spawn one on the next
/// tick if the cell still has Build paint and no structure yet.
#[allow(clippy::type_complexity)]
pub fn worker_build_assignment_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    mut slots: ResMut<SoftWorkSlots>,
    workers: Query<
        (Entity, &Transform, &Commitment, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            Without<BuildAssignment>,
            Without<BuildProgress>,
            Without<DirectMovementComponent>,
        ),
    >,
    build_sites: Query<(Entity, &Transform, &BuildSite)>,
    damaged_structures: Query<(Entity, &Transform, &Structure)>,
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

        // Find the nearest build target in the chosen cell.
        let Some(target) = find_nearest_build_target(
            candidate.cell,
            worker_pos,
            &build_sites,
            &damaged_structures,
        ) else {
            // No build target in the painted cell. The Build
            // Zone stays painted; the worker stays idle. A
            // future tick that auto-spawns a BuildSite (or a
            // new Structure degrades into repair) will pick
            // this worker up.
            continue;
        };

        // Resolve the world position of the target so the
        // worker can walk to it.
        let target_pos = if let Ok((_, t, _)) = build_sites.get(target) {
            t.translation.truncate()
        } else if let Ok((_, t, _)) = damaged_structures.get(target) {
            t.translation.truncate()
        } else {
            continue;
        };

        slots.occupy(candidate.cell, IntentKind::Build);
        commands.entity(entity).insert((
            BuildAssignment {
                cell: candidate.cell,
                target,
            },
            DirectMovementComponent { xy: target_pos },
        ));
    }
}

fn find_nearest_build_target(
    cell: IVec2,
    worker_pos: Vec2,
    build_sites: &Query<(Entity, &Transform, &BuildSite)>,
    damaged_structures: &Query<(Entity, &Transform, &Structure)>,
) -> Option<Entity> {
    let mut best_site: Option<(f32, Entity)> = None;
    for (entity, transform, site) in build_sites.iter() {
        if site.cell != cell {
            continue;
        }
        let d = worker_pos.distance(transform.translation.truncate());
        if best_site.is_none_or(|(bd, _)| d < bd) {
            best_site = Some((d, entity));
        }
    }
    let mut best_damaged: Option<(f32, Entity)> = None;
    for (entity, transform, structure) in damaged_structures.iter() {
        if structure.is_full_health() {
            continue;
        }
        if world_to_cell(transform.translation.truncate()) != cell {
            continue;
        }
        let d = worker_pos.distance(transform.translation.truncate());
        if best_damaged.is_none_or(|(bd, _)| d < bd) {
            best_damaged = Some((d, entity));
        }
    }
    // Sites take priority over repair. Fresh construction is
    // more "useful" than repair (the structure does not exist
    // yet), so a worker in a cell with both a site and a
    // damaged structure finishes the build first.
    best_site
        .map(|(_, e)| e)
        .or_else(|| best_damaged.map(|(_, e)| e))
}

/// Detect a worker that has arrived at its assigned build target
/// and start the work phase. The `Without<BuildProgress>` filter
/// makes arrival idempotent: the same tick cannot fire twice.
///
/// The arrival threshold matches the gather/haul pattern: the
/// movement system removes `DirectMovementComponent` when the
/// bot is within `STOP_THRESHOLD` of its target, and that
/// removal is the trigger this system waits for.
#[allow(clippy::type_complexity)]
pub fn worker_build_arrive_system(
    mut commands: Commands,
    workers: Query<
        (Entity, &Transform, &BuildAssignment),
        (
            With<Nanobot>,
            With<BuildAssignment>,
            Without<DirectMovementComponent>,
            Without<BuildProgress>,
        ),
    >,
    build_sites: Query<&Transform, With<BuildSite>>,
    structures: Query<&Transform, With<Structure>>,
) {
    for (entity, transform, assignment) in &workers {
        let target_transform = if let Ok(t) = build_sites.get(assignment.target) {
            t
        } else if let Ok(t) = structures.get(assignment.target) {
            t
        } else {
            // Target disappeared (e.g. a future system removed
            // a BuildSite). Release the slot and drop the
            // assignment; the worker idles.
            commands.entity(entity).remove::<BuildAssignment>();
            continue;
        };
        let distance = transform
            .translation
            .truncate()
            .distance(target_transform.translation.truncate());
        if distance <= STOP_THRESHOLD {
            commands.entity(entity).insert(BuildProgress {
                cell: assignment.cell,
                target: assignment.target,
            });
        }
    }
}

/// Worker build work system. For each worker with a
/// [`BuildProgress`], pull material from the nearest local
/// stockpile and either advance the build site or repair the
/// structure. Build vs repair is decided by the target's
/// current component: a `BuildSite` is constructed, a
/// `Structure` is repaired. The worker keeps working until the
/// site completes, the structure is at full health, or the
/// nearest stockpile has no more material (the worker then
/// waits in place for haulers to deliver more).
#[allow(clippy::type_complexity)]
pub fn worker_build_work_system(
    mut commands: Commands,
    mut slots: ResMut<SoftWorkSlots>,
    mut workers: Query<(Entity, &Transform, &BuildProgress), (With<Nanobot>, With<BuildProgress>)>,
    mut build_sites: Query<(Entity, &mut BuildSite)>,
    mut structures: Query<&mut Structure>,
    mut stockpiles: Query<(Entity, &mut Stockpile, &Transform)>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, transform, progress) in &mut workers {
        let worker_pos = transform.translation.truncate();

        // Construction branch: a worker at a BuildSite consumes
        // material and the site tracks progress.
        if let Ok((site_entity, mut site)) = build_sites.get_mut(progress.target) {
            if site.is_complete() {
                // Another worker finished the site between
                // ticks.
                promote_site_to_structure(&mut commands, site_entity, site.kind);
                release_build_worker(&mut commands, &mut slots, entity, progress.cell);
                continue;
            }
            if let Some(amount) = take_one_from_nearest_stockpile(
                worker_pos,
                &mut stockpiles,
                &mut ledger,
                ResourceKind::Minerals,
            ) {
                site.consumed_materials =
                    (site.consumed_materials + amount).min(site.required_materials);
                if site.is_complete() {
                    promote_site_to_structure(&mut commands, site_entity, site.kind);
                    release_build_worker(&mut commands, &mut slots, entity, progress.cell);
                }
            }
            // No local material: the worker stays at the site
            // and waits for haulers to deliver more.
            continue;
        }

        // Repair branch: a worker at a damaged Structure
        // consumes material and adds to health.
        if let Ok(mut structure) = structures.get_mut(progress.target) {
            if structure.is_full_health() {
                release_build_worker(&mut commands, &mut slots, entity, progress.cell);
                continue;
            }
            if let Some(amount) = take_one_from_nearest_stockpile(
                worker_pos,
                &mut stockpiles,
                &mut ledger,
                ResourceKind::Minerals,
            ) {
                let heal = amount * BUILD_HEALTH_PER_MATERIAL;
                structure.health = (structure.health + heal).min(STRUCTURE_MAX_HEALTH);
                if structure.is_full_health() {
                    release_build_worker(&mut commands, &mut slots, entity, progress.cell);
                }
            }
        } else {
            // Target disappeared (e.g. destroyed by a future
            // system).
            release_build_worker(&mut commands, &mut slots, entity, progress.cell);
        }
    }
}

/// Release the build cell's soft work slot and clear the
/// worker's build markers. Used at every transition out of
/// working state so the cleanup logic lives in one place.
fn release_build_worker(
    commands: &mut Commands,
    slots: &mut ResMut<SoftWorkSlots>,
    entity: Entity,
    cell: IVec2,
) {
    slots.release(cell, IntentKind::Build);
    commands
        .entity(entity)
        .remove::<BuildAssignment>()
        .remove::<BuildProgress>();
}

/// Take one unit of `kind` from the stockpile closest to
/// `worker_pos` that has at least one unit. Returns the amount
/// taken (always 1 in the first implementation) or `None` when
/// no stockpile in the world has material.
///
/// The first pass scans the query to find the nearest matching
/// entity; the second pass mutates it. The borrow on the query
/// is released between passes so the immutable scan and the
/// mutable update do not overlap.
fn take_one_from_nearest_stockpile(
    worker_pos: Vec2,
    stockpiles: &mut Query<(Entity, &mut Stockpile, &Transform)>,
    ledger: &mut ResMut<ResourceLedger>,
    kind: ResourceKind,
) -> Option<u32> {
    let target = stockpiles
        .iter()
        .filter(|(_, s, _)| s.kind == kind && s.amount > 0)
        .min_by(|(_, _, ta), (_, _, tb)| {
            worker_pos
                .distance(ta.translation.truncate())
                .partial_cmp(&worker_pos.distance(tb.translation.truncate()))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(e, _, _)| e)?;
    let (_, mut stockpile, _) = stockpiles.get_mut(target).ok()?;
    let taken = 1u32.min(stockpile.amount);
    stockpile.amount -= taken;
    ledger.remove(stockpile.kind, taken);
    Some(taken)
}

/// Promote a finished `BuildSite` to a completed `Structure` at
/// the same entity. The Transform is preserved by Bevy's
/// component-merge semantics. The completion transition is the
/// visible end of the "construct" contract.
fn promote_site_to_structure(commands: &mut Commands, site_entity: Entity, kind: StructureKind) {
    commands.entity(site_entity).remove::<BuildSite>();
    commands.entity(site_entity).insert(Structure::new(kind));
}

/// Plugin that wires the build systems into the Update schedule.
/// The chain runs after `move_velocity_system` so the movement
/// system has already pruned arrived bots (which is the trigger
/// the arrive system waits for).
pub struct BuildPlugin;

impl Plugin for BuildPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                structure_auto_creation_system,
                worker_build_assignment_system,
                worker_build_arrive_system,
                worker_build_work_system,
            )
                .chain()
                .after(crate::nanobot::move_velocity_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! (auto-creation, assignment, material consumption,
    //! repair, soft slots) are covered by
    //! `tests/build_zone_behavior.rs`.

    use super::*;

    #[test]
    fn structure_starts_at_full_health() {
        let s = Structure::new(StructureKind::Basic);
        assert_eq!(s.health, STRUCTURE_MAX_HEALTH);
        assert!(s.is_full_health());
    }

    #[test]
    fn structure_reports_full_health_correctly_after_damage() {
        let mut s = Structure::new(StructureKind::Basic);
        s.health = STRUCTURE_MAX_HEALTH - 1;
        assert!(!s.is_full_health());
        s.health = 0;
        assert!(!s.is_full_health());
        s.health = STRUCTURE_MAX_HEALTH;
        assert!(s.is_full_health());
    }

    #[test]
    fn build_site_starts_with_zero_consumed_materials() {
        let site = BuildSite::new(IVec2::new(0, 0), StructureKind::Basic);
        assert_eq!(site.cell, IVec2::new(0, 0));
        assert_eq!(site.kind, StructureKind::Basic);
        assert_eq!(site.required_materials, BUILD_REQUIRED_MATERIALS);
        assert_eq!(site.consumed_materials, 0);
        assert!(!site.is_complete());
    }

    #[test]
    fn build_site_completes_when_consumed_equals_required() {
        let mut site = BuildSite::new(IVec2::new(1, 1), StructureKind::Basic);
        site.consumed_materials = site.required_materials;
        assert!(site.is_complete());
        // Past the budget still counts as complete.
        site.consumed_materials = site.required_materials + 5;
        assert!(site.is_complete());
    }

    #[test]
    fn structure_kind_default_is_basic() {
        assert_eq!(StructureKind::default(), StructureKind::Basic);
    }

    #[test]
    fn build_budget_matches_max_health() {
        // A fully degraded structure must be repairable with
        // the same material budget as a fresh build so the
        // "construct or repair" contract is symmetric.
        const { assert!(BUILD_REQUIRED_MATERIALS * BUILD_HEALTH_PER_MATERIAL == STRUCTURE_MAX_HEALTH) };
    }
}
