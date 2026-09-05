//! Planned Structure foundation for Automatic Construction.
//!
//! Issue #21 contract: a Planned Structure is the visible,
//! owner-scoped, not-yet-built support structure that lives
//! between "automatic construction picked a kind" and "the
//! support structure is finished and usable". The slice
//! covers the foundation plus a minimal demo completion path
//! so the lifecycle is verifiable end-to-end:
//!
//! 1. A demand system creates a [`PlannedStructure`] in a
//!    cell. It is visible from the moment it exists, with a
//!    distinct "planned" visual (see [`planned_visual_color`]).
//! 2. A single Worker can claim the planned structure by
//!    becoming its `active_worker`. Other Workers see a
//!    claimed planned structure as unavailable.
//! 3. While the worker is at the site, build progress is
//!    `work_remaining` ticks of worker time. V1 consumes no
//!    minerals; the only cost is worker time.
//! 4. When `work_remaining` reaches 0, the planned structure
//!    is replaced by the appropriate completed structure for
//!    its kind. The foundation slice ships four kinds:
//!    [`PlannedKind::SourceStockpile`] and
//!    [`PlannedKind::SinkStockpile`] (both complete into a
//!    [`crate::resources::Stockpile`] stamped with the
//!    matching [`crate::resources::StockpileRole`]),
//!    [`PlannedKind::ProductionFacility`] (completes into a
//!    [`crate::nanobot::ProductionFacility`], issue #27), and
//!    [`PlannedKind::Charger`] (completes into a
//!    [`crate::nanobot::Charger`], issue #28).
//!
//! State machine carried on the worker by marker components:
//!
//! ```text
//!   Idle -> (claim system) -> Moving (PlannedStructureClaim + DMC)
//!   Moving -> (arrive system) -> Working (PlannedStructureProgress)
//!   Working -> (work system) -> Working (work_remaining -= 1 each tick)
//!   Working -> (work_remaining == 0) -> Idle (planned promoted)
//! ```
//!
//! The plan/complete boundary uses Bevy component-merge
//! semantics: the planned structure's `Transform` is preserved
//! on completion, and the `PlannedStructure` component is
//! swapped for the completed structure's components. The
//! `active_worker` is cleared during completion so the worker
//! returns to the idle state without an extra system
//! release path.
//!
//! Visual distinction: planned structures render with a
//! semi-transparent planned color ([`planned_visual_color`])
//! and a fixed footprint size. Completed structures
//! (Source Stockpiles, Sink Stockpiles, Production
//! Facilities, and Chargers) render with a different
//! (full-opacity) color and the same footprint. Tests can
//! pin the distinction by reading the `Sprite` `color`
//! channel, or by reading the component (`PlannedStructure`
//! vs `Stockpile` + `StockpileRole`).

use crate::navigation::Obstacle;
use std::collections::HashMap;

use bevy::prelude::*;

use crate::nanobot::InteractionRegion;

use crate::GAMEPLAY_SPRITE_Z;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::{DirectMovementComponent, Nanobot, Swarm, SwarmId, SwarmMember};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::production::{OwnerSwarm, ProductionFacility};
use crate::resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole};
use crate::structure_sprites::{StructureSprites, StructureVisual, StructureVisualState};

/// Number of worker-time ticks required to finish a planned
/// structure. V1 consumes no minerals; the only cost is this
/// counter decrementing each tick the worker is at the
/// planned structure. Picked to be small enough that a single
/// worker finishes the demo build in a handful of ticks so
/// the test math is obvious.
pub const DEFAULT_PLANNED_WORK_TICKS: u32 = 5;

/// Local sprite size shared by planned and completed visuals. The aligned
/// transform determines the physical whole-cell rectangle.
pub const PLANNED_STRUCTURE_FOOTPRINT: f32 = crate::navigation::STRUCTURE_SPRITE_SIZE;

/// Planned kind the foundation slice implements.
///
/// The PRD names Source Stockpile, Sink Stockpile, Production
/// Facility, and Charger as the eventual kinds. The
/// foundation slice ships the lifecycle plus the Source and
/// Sink Stockpile kinds (issue #26 migrates Sink Stockpiles
/// onto the planned-structure lifecycle). Issue #27 migrates
/// Production Facilities onto the same lifecycle. Issue #28
/// migrates Chargers onto the same lifecycle. All four
/// PRD-named kinds now live on the shared foundation.
///
/// All variants are data-less so [`PlannedKind::ALL`] can stay
/// a `const` array (the future-target kind for a planned
/// Production Facility lives on a sidecar component,
/// [`PlannedProductionTarget`], instead of on the enum).
#[derive(Debug, Component, Default, Clone, Copy, PartialEq, Eq)]
pub enum PlannedKind {
    /// Completes into a [`Stockpile`] (Source Stockpile in the
    /// glossary's role). This is the foundation's demo kind.
    #[default]
    SourceStockpile,
    /// Completes into a [`Stockpile`] marked as a Sink
    /// Stockpile in the base logistics network. Lives in a
    /// `Build`-painted cell (the Build Zone is the placement
    /// constraint); the demand system plans one per Build
    /// cell on the Build Zone owner's side, and a Worker
    /// builds it through the same lifecycle as the Source
    /// Stockpile.
    SinkStockpile,
    /// Completes into a [`ProductionFacility`]. The kind
    /// emerges from production demand pressure (issue #27)
    /// rather than from raw Build paint: the auto-creation
    /// system plans one inside an owned Build Zone cell when
    /// existing capacity is too busy. The first production
    /// target the completed facility should pick lives on a
    /// sidecar [`PlannedProductionTarget`] component, not on
    /// the enum itself, so the planned kind stays a
    /// const-friendly tag.
    ProductionFacility,
    /// Completes into a [`crate::nanobot::Charger`]. The
    /// kind emerges when a low-Charge Defender has no available
    /// completed or pending swarm-wide capacity. The nearest
    /// non-overlapping site in owned Defend paint receives the
    /// plan. A Worker then builds it through the
    /// same lifecycle as the other kinds; the completed
    /// charger uses the default `Charger::new(cell)` shape
    /// so the existing charge sustain loop picks it up
    /// without any further wiring. The planned kind stays
    /// a const-friendly tag and does not carry the cell on
    /// the enum because `PlannedStructure` already records
    /// it.
    Charger,
}

impl PlannedKind {
    /// Stable per-kind index in `[0, COUNT)`. Used to size
    /// tables and to give a deterministic order to iteration.
    pub const fn index(self) -> usize {
        match self {
            PlannedKind::SourceStockpile => 0,
            PlannedKind::SinkStockpile => 1,
            PlannedKind::ProductionFacility => 2,
            PlannedKind::Charger => 3,
        }
    }

    /// Number of distinct planned kinds the foundation slice
    /// models.
    pub const COUNT: usize = 4;

    /// Every planned kind in stable declaration order. Useful
    /// for tests and future "iterate every kind" loops.
    pub const ALL: [PlannedKind; Self::COUNT] = [
        PlannedKind::SourceStockpile,
        PlannedKind::SinkStockpile,
        PlannedKind::ProductionFacility,
        PlannedKind::Charger,
    ];
}

/// A visible, not-yet-built support structure. Lives in a
/// single intent cell. The `active_worker` field is the
/// one-Worker reservation the lifecycle promises: it is
/// `Some(worker)` while a Worker is committed to the build,
/// and `None` while the planned structure is unclaimed.
///
/// `work_remaining` is the build budget in worker-time
/// ticks. The work system decrements it by 1 each tick the
/// assigned worker is in working state; reaching 0 triggers
/// the promotion to the completed structure.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedStructure {
    pub kind: PlannedKind,
    pub cell: IVec2,
    pub work_remaining: u32,
    pub active_worker: Option<Entity>,
}

impl PlannedStructure {
    /// Build a fresh planned structure of `kind` in `cell` with
    /// the default work budget and no active worker.
    pub fn new(kind: PlannedKind, cell: IVec2) -> Self {
        Self {
            kind,
            cell,
            work_remaining: DEFAULT_PLANNED_WORK_TICKS,
            active_worker: None,
        }
    }

    /// True when no Worker has claimed this planned structure.
    /// The "at most one Worker" contract is enforced by the
    /// claim system only targeting unclaimed planned
    /// structures, so a `true` return is the only state in
    /// which a new claim is allowed.
    pub fn is_unclaimed(&self) -> bool {
        self.active_worker.is_none()
    }

    /// True when build progress has finished and the planned
    /// structure is ready to be promoted to the completed
    /// structure for its kind.
    pub fn is_complete(&self) -> bool {
        self.work_remaining == 0
    }
}

/// Release a plan reservation when its Worker no longer exists or no longer
/// carries lifecycle state for that plan. Reconciliation runs before
/// opportunity projection so the same allocation pass can offer the plan to
/// another Worker after death or lease revocation.
fn release_stale_planned_workers_system(
    mut planned: Query<(Entity, &mut PlannedStructure)>,
    workers: Query<
        (
            Option<&PlannedStructureClaim>,
            Option<&PlannedStructureProgress>,
        ),
        With<Nanobot>,
    >,
) {
    for (planned_entity, mut planned) in &mut planned {
        let active_worker_is_valid = planned.active_worker.is_some_and(|worker| {
            workers.get(worker).is_ok_and(|(claim, progress)| {
                claim.is_some_and(|claim| claim.target == planned_entity)
                    || progress.is_some_and(|progress| progress.target == planned_entity)
            })
        });
        if planned.active_worker.is_some() && !active_worker_is_valid {
            planned.active_worker = None;
        }
    }
}

/// Color the planned-structure visual uses. Semi-transparent
/// so the player can still see the underlying map and so the
/// structure is clearly "not finished yet" at a glance. The
/// completed structure (Source or Sink Stockpile) uses
/// [`completed_visual_color`] instead, so the visual flip on
/// completion is visible even without a sprite swap.
pub const fn planned_visual_color() -> Color {
    Color::srgba(0.6, 0.6, 0.7, 0.5)
}

/// Color the completed structure visual uses (Source and
/// Sink Stockpiles both). Full opacity and a different hue
/// from the planned visual so the promotion moment is
/// visible. Tests can pin the distinction by reading the
/// `Sprite` `color` field.
pub const fn completed_visual_color() -> Color {
    Color::srgba(0.2, 0.6, 0.3, 1.0)
}

/// Marker on a Worker that has claimed a planned structure.
/// `target` is the [`PlannedStructure`] entity. The arrive
/// system reads the same `target` from this component so the
/// work system does not need to look up the original
/// assignment.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedStructureClaim {
    pub cell: IVec2,
    pub target: Entity,
}

/// Marker on a Worker that is at its claimed planned
/// structure and is consuming worker time to build it. The
/// work system decrements `work_remaining` on the planned
/// structure each tick the worker has this marker.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedStructureProgress {
    pub cell: IVec2,
    pub target: Entity,
}

/// Planning-time snapshot of the type most under target when a
/// [`PlannedKind::ProductionFacility`] plan is created.
///
/// The sidecar stays separate so [`PlannedKind`] remains data-less. Promotion
/// removes it and creates an idle, empty facility; physical input must pay the
/// first cycle before the normal picker chooses any type.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedProductionTarget(pub NanobotType);

/// Build-painted cells where a consumer may place its local Sink Stockpile.
/// Both planning and collapse recovery use this helper so ownership and
/// consumer-local topology cannot diverge.
pub(crate) fn sink_stockpile_zone_cells(
    grid: &IntentGrid,
    consumer_cell: IVec2,
    consumer_owner: Option<Entity>,
    swarm_by_id: &HashMap<SwarmId, Entity>,
) -> Vec<IVec2> {
    let Some(intent_cell) = grid.cell(consumer_cell) else {
        return Vec::new();
    };
    if !intent_cell.has(IntentKind::Build) {
        return Vec::new();
    }
    let painted_owner = intent_cell
        .owner(IntentKind::Build)
        .and_then(|id| swarm_by_id.get(&id).copied());
    if consumer_owner.is_some() && painted_owner.is_some() && consumer_owner != painted_owner {
        return Vec::new();
    }

    let mut zone_cells = Vec::new();
    for dx in -1..=1 {
        for dy in -1..=1 {
            let cell = consumer_cell + IVec2::new(dx, dy);
            let Some(intent) = grid.cell(cell) else {
                continue;
            };
            if !intent.has(IntentKind::Build) {
                continue;
            }
            let cell_owner = intent
                .owner(IntentKind::Build)
                .and_then(|id| swarm_by_id.get(&id).copied());
            if consumer_owner.is_some() && cell_owner.is_some() && cell_owner != consumer_owner {
                continue;
            }
            zone_cells.push(cell);
        }
    }
    zone_cells
}

/// Plan Sink Stockpiles only when sink-side storage has a real
/// nearby consumer. Raw Build paint is only a placement constraint:
/// it does not create construction demand by itself. A pending or
/// completed Production Facility in a Build cell asks for one Sink
/// Stockpile in its same-owner local 3x3 Build zone, without overlapping
/// deposits or other support structures. Chargers accept direct delivery and
/// do not create Sink demand.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sink_stockpile_demand_system(
    mut commands: Commands,
    access: super::construction_access::ConstructionAccess,
    grid: Res<IntentGrid>,
    structure_sprites: Res<StructureSprites>,
    planned: Query<(&PlannedStructure, &Transform, Option<&OwnerSwarm>)>,
    stockpiles: Query<(
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: Query<(&Transform, Option<&OwnerSwarm>), With<ProductionFacility>>,
    chargers: Query<(&Transform, Option<&OwnerSwarm>), With<crate::nanobot::Charger>>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    let mut access_layout = access.snapshot();
    let swarm_by_id: HashMap<SwarmId, Entity> = swarms.iter().map(|(e, id)| (*id, e)).collect();
    let mut obstacles: Vec<Obstacle> = deposits
        .iter()
        .map(|(deposit, transform)| {
            Obstacle::deposit(transform.translation.truncate(), deposit.radius)
        })
        .collect();
    for (_, transform, _, _) in &stockpiles {
        obstacles.push(Obstacle::structure(transform));
    }
    // Planned Structures of any kind are in the obstacle
    // list so a fresh Sink Stockpile cannot overlap a
    // pending Production Facility or Charger plan. Only
    // Production Facility plans satisfy sink-side demand:
    // chargers are direct-delivery terminals and do not
    // auto-plan Sink Stockpiles (ADR-0005).
    let mut demand_sites: Vec<(IVec2, Option<Entity>)> = Vec::new();
    for (planned_structure, transform, owner) in &planned {
        obstacles.push(Obstacle::structure(transform));
        if planned_structure.kind == PlannedKind::ProductionFacility {
            demand_sites.push((
                world_to_cell(transform.translation.truncate()),
                owner.map(|o| o.0),
            ));
        }
    }
    for (transform, _) in &facilities {
        obstacles.push(Obstacle::structure(transform));
    }
    for (transform, _) in &chargers {
        obstacles.push(Obstacle::structure(transform));
    }

    for (transform, owner) in &facilities {
        demand_sites.push((
            world_to_cell(transform.translation.truncate()),
            owner.map(|o| o.0),
        ));
    }
    // Chargers are direct-delivery terminals fed by haulers;
    // they deliberately do not create Sink Stockpile demand.
    // They stay in the obstacle list above so facility-side
    // sink plans cannot overlap them.
    demand_sites.sort_by_key(|(cell, _)| (cell.x, cell.y));
    demand_sites.dedup();

    let mut newly_planned: Vec<Vec2> = Vec::new();
    for (cell, owner) in demand_sites {
        let painted_owner = grid
            .cell(cell)
            .and_then(|intent| intent.owner(IntentKind::Build))
            .and_then(|id| swarm_by_id.get(&id).copied());
        let zone_cells = sink_stockpile_zone_cells(&grid, cell, owner, &swarm_by_id);
        if zone_cells.is_empty() {
            continue;
        }
        let in_zone = |cell: IVec2| zone_cells.contains(&cell);
        let sink_exists = stockpiles
            .iter()
            .any(|(_, transform, role, stockpile_owner)| {
                matches!(role, Some(StockpileRole::Sink))
                    && in_zone(world_to_cell(transform.translation.truncate()))
                    && (owner.is_none() || stockpile_owner.map(|o| o.0) == owner)
            })
            || planned
                .iter()
                .any(|(planned_structure, transform, plan_owner)| {
                    planned_structure.kind == PlannedKind::SinkStockpile
                        && in_zone(world_to_cell(transform.translation.truncate()))
                        && (owner.is_none() || plan_owner.map(|o| o.0) == owner)
                });
        if sink_exists {
            continue;
        }
        let mut local_obstacles = obstacles.clone();
        local_obstacles.extend(newly_planned.iter().map(|pos| Obstacle::planned(*pos)));
        let placement_swarm = owner
            .or(painted_owner)
            .and_then(|owner| swarms.get(owner).ok().map(|(_, id)| *id))
            .unwrap_or(SwarmId::PLAYER);
        let Some((placement_cell, placement_pos)) =
            crate::nanobot::placement::find_build_zone_placement_accepting(
                &zone_cells,
                &local_obstacles,
                26,
                |position| {
                    access.accepts(
                        &access_layout,
                        &grid,
                        placement_swarm,
                        PlannedKind::SinkStockpile,
                        position,
                    )
                },
            )
        else {
            continue;
        };
        access_layout.reserve(
            placement_swarm,
            crate::navigation::align_structure(Transform::from_translation(
                placement_pos.extend(0.0),
            )),
        );
        newly_planned.push(placement_pos);
        let mut entity_commands = commands.spawn((
            PlannedStructure::new(PlannedKind::SinkStockpile, placement_cell),
            planned_visual_components(
                PlannedKind::SinkStockpile,
                &structure_sprites,
                placement_pos,
            ),
        ));
        if let Some(owner) = owner.or(painted_owner) {
            entity_commands.insert(OwnerSwarm(owner));
        }
    }
}

/// For each idle Worker with no in-flight planned-structure
/// work, pick the nearest unclaimed [`PlannedStructure`] and
/// claim it.
///
/// The "at most one Worker" contract is enforced two ways.
/// The `is_unclaimed()` filter skips planned structures that
/// are already reserved. The local `claimed` set tracks
/// reservations written earlier in the same tick, so two
/// workers that both see a planned structure as unclaimed
/// on entry do not both claim it. [`Commands`] are deferred,
/// so the live query cannot see the new reservation until
/// the next system call -- the local set is what makes the
/// in-tick reservation visible. The reservation lives on
/// the planned structure itself (`active_worker = Some(worker)`)
/// so every other system that looks at planned structures
/// sees it without going through the worker's marker.
///
/// The "only Workers build" half of the lifecycle is
/// enforced by filtering on `NanobotType::Worker`: the
/// claim system pulls `&NanobotType` out of the query and
/// skips any nanobot that is not a Worker. Defenders and
/// Haulers do not claim planned structures; the
/// "defend" path's defenders stay on their cell and the
/// "hauler" path's haulers stay on their run. Issue #28
/// added the Worker filter so a Defend cell's defenders
/// do not accidentally claim a planned Charger.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_claim_system(
    mut commands: Commands,
    planned_structures: Query<(Entity, &Transform, &PlannedStructure, Option<&OwnerSwarm>)>,
    workers: Query<
        (Entity, &Transform, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            Without<PlannedStructureClaim>,
            Without<PlannedStructureProgress>,
            Without<DirectMovementComponent>,
        ),
    >,
    swarms: Query<&SwarmId>,
) {
    let mut claimed: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for (worker_entity, worker_transform, nanobot_type, swarm_member) in &workers {
        // The Planned Structure lifecycle is a Worker job:
        // only Workers carry material to a build site and
        // spend worker time on construction. Defenders and
        // Haulers are filtered out so a Defend cell's
        // defenders do not accidentally claim a planned
        // Charger and a busy Hauler does not get pulled
        // off its run to build a structure.
        if *nanobot_type != NanobotType::Worker {
            continue;
        }
        let worker_pos = worker_transform.translation.truncate();

        let mut best: Option<(f32, Entity, &PlannedStructure, Vec2)> = None;
        for (planned_entity, planned_transform, planned, owner) in &planned_structures {
            if !planned.is_unclaimed() {
                continue;
            }
            if claimed.contains(&planned_entity) {
                continue;
            }
            if !planned_owner_matches_worker(owner, &swarms, swarm_member.0) {
                continue;
            }
            let distance = worker_pos.distance(planned_transform.translation.truncate());
            if best.is_none_or(|(bd, _, _, _)| distance < bd) {
                best = Some((
                    distance,
                    planned_entity,
                    planned,
                    planned_transform.translation.truncate(),
                ));
            }
        }
        let Some((_distance, planned_entity, planned, _planned_pos)) = best else {
            continue;
        };
        let Ok((_, target_transform, _, _)) = planned_structures.get(planned_entity) else {
            continue;
        };
        claimed.insert(planned_entity);

        commands.entity(planned_entity).insert(PlannedStructure {
            active_worker: Some(worker_entity),
            ..*planned
        });
        commands.entity(worker_entity).insert((
            PlannedStructureClaim {
                cell: planned.cell,
                target: planned_entity,
            },
            InteractionRegion::structure(target_transform).movement_from(worker_pos),
        ));
    }
}

fn planned_owner_matches_worker(
    owner: Option<&OwnerSwarm>,
    swarms: &Query<&SwarmId>,
    worker_swarm: SwarmId,
) -> bool {
    match owner {
        None => true,
        Some(OwnerSwarm(owner_entity)) => swarms
            .get(*owner_entity)
            .is_ok_and(|owner_id| *owner_id == worker_swarm),
    }
}

/// Detect a worker that has arrived at its claimed planned
/// structure and start the work phase. The
/// `Without<PlannedStructureProgress>` filter makes arrival
/// idempotent: the same tick cannot fire twice.
///
/// Arrival requires the same exterior region used for movement and ongoing work.
/// Displaced workers reapproach while retaining their claim.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_arrive_system(
    mut commands: Commands,
    workers: Query<
        (Entity, &Transform, &PlannedStructureClaim),
        (
            With<Nanobot>,
            With<PlannedStructureClaim>,
            Without<DirectMovementComponent>,
            Without<PlannedStructureProgress>,
        ),
    >,
    planned_transforms: Query<&Transform, With<PlannedStructure>>,
) {
    for (worker_entity, worker_transform, claim) in &workers {
        let Ok(planned_transform) = planned_transforms.get(claim.target) else {
            // Target disappeared (e.g. promoted by another
            // worker, or removed by a future cleanup system).
            // Drop the claim; the worker idles.
            commands
                .entity(worker_entity)
                .remove::<PlannedStructureClaim>();
            continue;
        };
        let region = InteractionRegion::structure(planned_transform);
        let position = worker_transform.translation.truncate();
        if region.contains(position) {
            commands
                .entity(worker_entity)
                .insert(PlannedStructureProgress {
                    cell: claim.cell,
                    target: claim.target,
                });
        } else {
            commands
                .entity(worker_entity)
                .insert(region.movement_from(position));
        }
    }
}

/// Apply construction work, then release its Worker while the site clears.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_work_system(
    mut commands: Commands,
    workers: Query<(Entity, &Transform, &PlannedStructureProgress), With<Nanobot>>,
    mut planned: Query<(&mut PlannedStructure, &Transform)>,
) {
    for (worker, transform, progress) in &workers {
        let Ok((mut plan, site)) = planned.get_mut(progress.target) else {
            release_planned_worker(&mut commands, worker);
            continue;
        };
        let position = transform.translation.truncate();
        let region = InteractionRegion::structure(site);
        if !region.contains(position) {
            commands
                .entity(worker)
                .insert(region.movement_from(position));
            continue;
        }
        plan.work_remaining = plan.work_remaining.saturating_sub(1);
        if plan.is_complete() {
            plan.active_worker = None;
            commands
                .entity(progress.target)
                .insert(crate::nanobot::StructureClearing {
                    builder_position: position,
                    validated_layout: None,
                });
            release_planned_worker(&mut commands, worker);
        }
    }
}

fn release_planned_worker(commands: &mut Commands, worker: Entity) {
    commands
        .entity(worker)
        .remove::<PlannedStructureClaim>()
        .remove::<PlannedStructureProgress>()
        .remove::<crate::nanobot::RegionalLease>();
}

/// Promote a finished [`PlannedStructure`] to the completed
/// structure for its kind, at the planned structure's world
/// position. The promotion removes the `PlannedStructure`
/// component, swaps the planned visual for the completed
/// visual, and stamps the matching completion payload on
/// the completed entity. The `Transform` is preserved by
/// Bevy's component-merge semantics.
///
/// The visual flip is shared by every kind (the completed
/// sprite + transform at `world_pos`), so
/// [`completed_visual_bundle`] factors it out. The
/// per-kind completion payload differs:
///
/// - [`PlannedKind::SourceStockpile`] and
///   [`PlannedKind::SinkStockpile`] both complete into an
///   empty [`Stockpile`] buffer, with
///   [`StockpileRole::Source`] or [`StockpileRole::Sink`]
///   respectively.
/// - [`PlannedKind::ProductionFacility`] completes into an empty, idle
///   [`ProductionFacility`]. `OwnerSwarm` is preserved, while the planning
///   target is removed. Logistics must deliver a complete cycle cost before
///   the normal production picker starts work.
/// - [`PlannedKind::Charger`] completes into an empty
///   [`crate::nanobot::Charger`] with default capacity and radius.
///   `OwnerSwarm` remains on the entity, preserving plan ownership.
///   `first_target` is unused for this kind; the
///   pre-existing test fixtures that pre-spawn a Charger
///   already establish the default-shape contract.
pub(crate) fn promote_planned_to_completion(
    commands: &mut Commands,
    planned_entity: Entity,
    kind: PlannedKind,
    transform: Transform,
    cell: IVec2,
    _first_target: Option<NanobotType>,
    structure_sprites: &StructureSprites,
) {
    let visual = completed_visual_bundle(kind, structure_sprites, transform);
    match kind {
        PlannedKind::SourceStockpile => {
            commands.entity(planned_entity).remove::<PlannedStructure>();
            commands.entity(planned_entity).insert((
                empty_mineral_stockpile(),
                StockpileRole::Source,
                visual,
            ));
        }
        PlannedKind::SinkStockpile => {
            commands.entity(planned_entity).remove::<PlannedStructure>();
            commands.entity(planned_entity).insert((
                empty_mineral_stockpile(),
                StockpileRole::Sink,
                visual,
            ));
        }
        PlannedKind::ProductionFacility => {
            // Completion creates an empty terminal. The normal production picker
            // chooses a type only after the hopper can pay the full cycle cost; a
            // planned target must never become a free first nanobot.
            let facility = ProductionFacility::new();
            commands
                .entity(planned_entity)
                .remove::<PlannedStructure>()
                .remove::<PlannedProductionTarget>();
            // A completed facility is a terminal consumer:
            // it owns its own input hopper (on
            // `ProductionFacility`) and is NOT a `Stockpile`.
            // Haulers fill the hopper via logistics leg 3
            // (sink stockpile -> facility); production
            // consumes exclusively from it. Keeping the
            // `Stockpile` component off the facility means
            // it never enters stockpile queries, so a
            // gather worker cannot dump a gather load into
            // it and a hauler cannot pick it as a
            // stockpile source/sink.
            commands.entity(planned_entity).insert((facility, visual));
        }
        PlannedKind::Charger => {
            // Completed chargers begin empty. OwnerSwarm remains on the
            // entity through Bevy component-merge semantics.
            let charger = crate::nanobot::Charger::new(cell);
            commands.entity(planned_entity).remove::<PlannedStructure>();
            commands.entity(planned_entity).insert((charger, visual));
        }
    }
}

/// The "build pending" visual shared by every planned kind.
/// Each auto-creation path pairs the [`PlannedStructure`]
/// component with this bundle, then completes by
/// [`completed_visual_bundle`] on promotion. Bevy replaces
/// the planned `Sprite` on `insert`, so the planned visual
/// does not leak through to the completed entity.
pub(crate) fn planned_visual_components(
    kind: PlannedKind,
    structure_sprites: &StructureSprites,
    world_pos: Vec2,
) -> (Sprite, Transform, StructureVisual) {
    let mut sprite = structure_sprites.sprite(kind, StructureVisualState::Planned);
    sprite.color = planned_visual_color();
    sprite.custom_size = Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT));
    (
        sprite,
        crate::navigation::align_structure(Transform::from_translation(
            world_pos.extend(GAMEPLAY_SPRITE_Z),
        )),
        StructureVisual::planned(kind),
    )
}

/// The "build finished" visual shared by every completed
/// planned-structure kind. Bevy replaces the planned
/// `Sprite` on `insert`, so the planned visual does not
/// leak through to the completed entity.
fn completed_visual_bundle(
    kind: PlannedKind,
    structure_sprites: &StructureSprites,
    transform: Transform,
) -> (Sprite, Transform, StructureVisual) {
    let mut sprite = structure_sprites.sprite(kind, StructureVisualState::Completed);
    sprite.color = completed_visual_color();
    sprite.custom_size = Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT));
    (sprite, transform, StructureVisual::completed(kind))
}

/// Default capacity for completed Source and Sink Stockpiles.
/// One full hauler load is one tenth of this buffer.
pub const DEFAULT_STOCKPILE_CAPACITY: u32 = 200;

/// Empty mineral buffer used by every completed planned kind
/// that needs a local `Stockpile`. Source and Sink Stockpiles
/// share capacity; their role marks logistics position, not
/// size.
fn empty_mineral_stockpile() -> Stockpile {
    Stockpile {
        kind: ResourceKind::Minerals,
        amount: 0,
        capacity: DEFAULT_STOCKPILE_CAPACITY,
        radius: 32.0,
    }
}

/// Plugin that wires the planned-structure systems into the
/// Update schedule. The chain runs after `move_velocity_system`
/// so the movement step has already pruned arrived bots (which
/// is the trigger the arrive system waits for), matching the
/// build plugin's chain order.
pub struct PlannedStructurePlugin;

impl Plugin for PlannedStructurePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::nanobot::construction_access::CancelledSites>()
            .add_systems(
                FixedUpdate,
                crate::nanobot::clearing::animate_cancelled_plans_system,
            )
            .add_systems(
                FixedUpdate,
                release_stale_planned_workers_system
                    .before(crate::nanobot::RegionalAllocationSet::Project),
            )
            .add_systems(
                FixedUpdate,
                (
                    sink_stockpile_demand_system,
                    worker_planned_structure_arrive_system,
                    worker_planned_structure_work_system,
                    crate::nanobot::clearing::clear_finished_structures_system,
                )
                    .chain()
                    .after(crate::nanobot::RegionalAllocationSet::Acquire)
                    .after(crate::nanobot::NanobotSimulationSet::Movement),
            );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! (auto-creation, claim, reservation, progress,
    //! completion, no-material-cost) are covered by
    //! `tests/behavior/planned_structure.rs`. The
    //! `PlannedKind` enum-shape contracts (default,
    //! `ALL`, stable indexes) live in
    //! `tests/behavior/sink_stockpile.rs` as part of the
    //! issue #26 acceptance suite.

    use super::*;

    #[test]
    fn planned_kind_iteration_and_indexes_cover_every_kind_in_order() {
        assert_eq!(
            PlannedKind::ALL,
            [
                PlannedKind::SourceStockpile,
                PlannedKind::SinkStockpile,
                PlannedKind::ProductionFacility,
                PlannedKind::Charger,
            ]
        );
        assert_eq!(PlannedKind::COUNT, 4);
        assert_eq!(PlannedKind::default(), PlannedKind::SourceStockpile);
        for (expected_index, kind) in PlannedKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.index(), expected_index);
        }
    }

    #[test]
    fn planned_structure_starts_unclaimed_with_full_budget() {
        let cell = IVec2::new(0, 0);
        let p = PlannedStructure::new(PlannedKind::SourceStockpile, cell);
        assert_eq!(p.kind, PlannedKind::SourceStockpile);
        assert_eq!(p.cell, cell);
        assert_eq!(p.work_remaining, DEFAULT_PLANNED_WORK_TICKS);
        assert!(p.is_unclaimed());
        assert!(!p.is_complete());
    }

    #[test]
    fn planned_structure_reports_unclaimed_only_when_no_worker() {
        let mut p = PlannedStructure::new(PlannedKind::SourceStockpile, IVec2::new(1, 1));
        assert!(p.is_unclaimed());
        // The reservation type is a plain `Option<Entity>`; the
        // test uses a dummy entity handle since the field's
        // contract is "is there a worker?", not "is the worker
        // still alive?".
        p.active_worker = Some(Entity::PLACEHOLDER);
        assert!(!p.is_unclaimed());
    }

    #[test]
    fn planned_structure_completes_only_when_budget_zero() {
        let mut p = PlannedStructure::new(PlannedKind::SourceStockpile, IVec2::new(0, 0));
        p.work_remaining = 1;
        assert!(!p.is_complete());
        p.work_remaining = 0;
        assert!(p.is_complete());
    }

    #[test]
    fn planned_visual_color_is_distinct_from_completed() {
        // The visual contract is "visibly distinct from
        // completed structures". The two colors must not be
        // identical, and the planned one must be at least
        // partially transparent so the player can see the
        // underlying map through the planned footprint.
        let planned = planned_visual_color();
        let completed = completed_visual_color();
        assert_ne!(
            planned, completed,
            "planned and completed visuals must be distinct"
        );
        // The planned visual carries an alpha < 1.0. We check
        // the alpha channel via the to_srgba helper, which
        // returns the four channels in canonical order.
        let planned_srgba = planned.to_srgba();
        assert!(
            planned_srgba.alpha < 1.0,
            "planned visual must be semi-transparent; got alpha={}",
            planned_srgba.alpha
        );
    }

    #[test]
    fn default_work_budget_is_small_for_fast_tests() {
        // The demo budget must be small enough that a single
        // worker finishes the build in a handful of ticks.
        // Pinning the value (rather than recomputing it) keeps
        // the test math obvious.
        const { assert!(DEFAULT_PLANNED_WORK_TICKS > 0) };
        const { assert!(DEFAULT_PLANNED_WORK_TICKS <= 32) };
    }

    #[test]
    fn footprint_is_a_finite_positive_square() {
        // The visual footprint must be positive so the planned
        // and completed sprites have a defined size. We do not
        // pin the value (it is a tuning parameter); the
        // invariant is the positive, finite, non-zero size.
        const { assert!(PLANNED_STRUCTURE_FOOTPRINT > 0.0) };
        const { assert!(PLANNED_STRUCTURE_FOOTPRINT.is_finite()) };
    }
}
