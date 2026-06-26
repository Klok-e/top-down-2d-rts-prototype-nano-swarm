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

use std::collections::HashMap;

use bevy::prelude::*;

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::NanobotType;
use crate::nanobot::components::{DirectMovementComponent, Nanobot, Swarm, SwarmId, SwarmMember};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::placement::{find_build_zone_placement, BUILDING_FOOTPRINT_RADIUS};
use crate::nanobot::production::{OwnerSwarm, ProductionFacility};
use crate::resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole};
use crate::structure_sprites::{StructureSprites, StructureVisual, StructureVisualState};
use crate::GAMEPLAY_SPRITE_Z;

/// Number of worker-time ticks required to finish a planned
/// structure. V1 consumes no minerals; the only cost is this
/// counter decrementing each tick the worker is at the
/// planned structure. Picked to be small enough that a single
/// worker finishes the demo build in a handful of ticks so
/// the test math is obvious.
pub const DEFAULT_PLANNED_WORK_TICKS: u32 = 5;

/// Footprint (world units) used for both the planned and
/// completed visuals. A square so the structure is clearly
/// bounded on the map and tests can compare positions without
/// doing per-axis math.
pub const PLANNED_STRUCTURE_FOOTPRINT: f32 = 64.0;

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
    /// kind emerges from Defend Zone demand (issue #28):
    /// when a Defend cell has defender load and the
    /// existing chargers (planned or completed) cannot
    /// cover it, a Planned Charger is planned at the cell's
    /// world center. A Worker then builds it through the
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

/// Sidecar on a `PlannedStructure` of
/// [`PlannedKind::ProductionFacility`]. Records the type the
/// completed facility should produce first, so the demand
/// layer can pre-allocate the kind that was most under target
/// at planning time. The completed `ProductionFacility`
/// starts with `current_target = Some(this type)`, so its
/// first pick cycle respects the original demand even if the
/// swarm's ratio has shifted between plan and completion.
///
/// The target lives on a sidecar component rather than as
/// data on [`PlannedKind`] so the enum (and its
/// `const` [`PlannedKind::ALL`] array) stay data-less and
/// trivially copyable. The promotion path reads this
/// component and removes it on completion; the production
/// pick/work systems then re-evaluate the target every cycle
/// as usual.
#[derive(Debug, Component, Clone, Copy)]
pub struct PlannedProductionTarget(pub NanobotType);

/// Plan Sink Stockpiles only when sink-side storage has a real
/// nearby consumer. Raw Build paint is only a placement constraint:
/// it does not create construction demand by itself. A pending or
/// completed Production Facility / Charger in a Build cell asks for
/// one local Sink Stockpile, placed in that same owned Build cell
/// without overlapping deposits or other support structures.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sink_stockpile_demand_system(
    mut commands: Commands,
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
    let swarm_by_id: HashMap<SwarmId, Entity> = swarms.iter().map(|(e, id)| (*id, e)).collect();
    let mut obstacles: Vec<(Vec2, f32)> = deposits
        .iter()
        .map(|(deposit, transform)| (transform.translation.truncate(), deposit.radius))
        .collect();
    for (_, transform, _, _) in &stockpiles {
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
    }
    // Planned Structures of any kind are in the obstacle
    // list so a fresh Sink Stockpile cannot overlap a
    // pending Production Facility or Charger plan. Only
    // Production Facility plans satisfy sink-side demand:
    // chargers are direct-delivery terminals and do not
    // auto-plan Sink Stockpiles (ADR-0005).
    let mut demand_sites: Vec<(IVec2, Option<Entity>)> = Vec::new();
    for (planned_structure, transform, owner) in &planned {
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
        if planned_structure.kind == PlannedKind::ProductionFacility {
            demand_sites.push((
                world_to_cell(transform.translation.truncate()),
                owner.map(|o| o.0),
            ));
        }
    }
    for (transform, _) in &facilities {
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
    }
    for (transform, _) in &chargers {
        obstacles.push((transform.translation.truncate(), BUILDING_FOOTPRINT_RADIUS));
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
        let Some(intent_cell) = grid.cell(cell) else {
            continue;
        };
        if !intent_cell.has(IntentKind::Build) {
            continue;
        }
        let painted_owner = intent_cell
            .owner(IntentKind::Build)
            .and_then(|id| swarm_by_id.get(&id).copied());
        if owner.is_some() && painted_owner.is_some() && owner != painted_owner {
            continue;
        }
        // Build the local Build Zone: the facility's own cell plus
        // its build-painted, same-owner neighbours (a 3x3 block).
        // The sink stockpile may land in any of these cells -- it
        // does not need the facility's exact cell, just the painted
        // zone near it (ADR-0005: dense-base starvation fix). The
        // non-overlap rule is still enforced by the placer; the
        // wider cell set just gives it more room to find a free
        // spot instead of silently starving a packed facility.
        let mut zone_cells: Vec<IVec2> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let nc = IVec2::new(cell.x + dx, cell.y + dy);
                let Some(nc_intent) = grid.cell(nc) else {
                    continue;
                };
                if !nc_intent.has(IntentKind::Build) {
                    continue;
                }
                let nc_owner = nc_intent
                    .owner(IntentKind::Build)
                    .and_then(|id| swarm_by_id.get(&id).copied());
                if owner.is_some() && nc_owner.is_some() && nc_owner != owner {
                    continue;
                }
                zone_cells.push(nc);
            }
        }
        if zone_cells.is_empty() {
            // No build-painted zone around this facility: skip
            // rather than force placement outside a Build Zone.
            continue;
        }
        let in_zone = |c: IVec2| zone_cells.contains(&c);
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
        local_obstacles.extend(
            newly_planned
                .iter()
                .map(|pos| (*pos, BUILDING_FOOTPRINT_RADIUS)),
        );
        let Some((placement_cell, placement_pos)) =
            find_build_zone_placement(&zone_cells, &local_obstacles, 26)
        else {
            continue;
        };
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
        let Some((_distance, planned_entity, planned, planned_pos)) = best else {
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
            // Issue #38 / ADR-0004: stop on the building
            // footprint's physical edge so the worker
            // lands at the planned structure's centre,
            // matching the arrive guard's radius-based
            // check below.
            DirectMovementComponent {
                xy: planned_pos,
                stop_radius: BUILDING_FOOTPRINT_RADIUS,
            },
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
/// The arrival threshold matches the building footprint
/// (issue #38 / ADR-0004): the same extent the
/// `DirectMovementComponent::stop_radius` the claim system
/// passes, and the same extent the build chain's arrive
/// guard uses. The guard is not redundant: a bot whose
/// `DirectMovementComponent` was stripped elsewhere (e.g.
/// the `ProgressChecker` stuck-timeout in `move_system`)
/// cannot produce a false arrival past the physical
/// extent, and the resume branch below re-issues a DMC
/// when a bot is between the building edge and a tight
/// arrival threshold.
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
        let distance = worker_transform
            .translation
            .truncate()
            .distance(planned_transform.translation.truncate());
        if distance <= BUILDING_FOOTPRINT_RADIUS {
            commands
                .entity(worker_entity)
                .insert(PlannedStructureProgress {
                    cell: claim.cell,
                    target: claim.target,
                });
        } else {
            // Resume branch (issue #38 / ADR-0004): the
            // `Without<DirectMovementComponent>` filter
            // guarantees the worker has no DMC, so
            // re-issue one with the same extent the
            // claim path uses. A bot nudged past the
            // footprint by separation force walks
            // back instead of stalling without a
            // movement command.
            commands
                .entity(worker_entity)
                .insert(DirectMovementComponent {
                    xy: planned_transform.translation.truncate(),
                    stop_radius: BUILDING_FOOTPRINT_RADIUS,
                });
        }
    }
}

/// Worker planned-structure work system. For each worker with
/// a [`PlannedStructureProgress`], decrement the planned
/// structure's `work_remaining` by 1 and, on the tick it
/// reaches 0, promote the planned structure to its completed
/// form.
///
/// The worker stays at the site until the build finishes or
/// the planned structure is removed; the reservation is
/// cleared on promotion so the worker returns to idle. V1
/// does not consume any minerals, so the resource ledger and
/// local stockpiles are untouched.
#[allow(clippy::type_complexity)]
pub fn worker_planned_structure_work_system(
    mut commands: Commands,
    structure_sprites: Res<StructureSprites>,
    workers: Query<
        (Entity, &PlannedStructureProgress),
        (With<Nanobot>, With<PlannedStructureProgress>),
    >,
    mut planned: Query<(
        Entity,
        &mut PlannedStructure,
        &Transform,
        Option<&PlannedProductionTarget>,
    )>,
) {
    for (worker_entity, progress) in &workers {
        let Ok((planned_entity, mut planned_state, planned_transform, first_target)) =
            planned.get_mut(progress.target)
        else {
            // Target disappeared. Release the worker; the
            // planned structure is gone so the worker has
            // nothing to do.
            commands
                .entity(worker_entity)
                .remove::<PlannedStructureClaim>()
                .remove::<PlannedStructureProgress>();
            continue;
        };
        let first_target = first_target.copied().map(|t| t.0);

        if planned_state.is_complete() {
            // Another worker (or a future system) already
            // finished this planned structure between ticks.
            // Promote defensively and release the worker.
            promote_planned_to_completion(
                &mut commands,
                planned_entity,
                planned_state.kind,
                planned_transform.translation.truncate(),
                planned_state.cell,
                first_target,
                &structure_sprites,
            );
            release_planned_worker(&mut commands, worker_entity);
            continue;
        }

        planned_state.work_remaining = planned_state.work_remaining.saturating_sub(1);
        if planned_state.is_complete() {
            promote_planned_to_completion(
                &mut commands,
                planned_entity,
                planned_state.kind,
                planned_transform.translation.truncate(),
                planned_state.cell,
                first_target,
                &structure_sprites,
            );
            release_planned_worker(&mut commands, worker_entity);
        }
    }
}

/// Release a worker that was building a planned structure:
/// clear both the claim and the progress markers so the
/// worker returns to the idle state. The build cell's slot is
/// not modelled for v1 (the planned structure is consumed
/// before the worker is free), so there is no slot to
/// release.
fn release_planned_worker(commands: &mut Commands, worker_entity: Entity) {
    commands
        .entity(worker_entity)
        .remove::<PlannedStructureClaim>()
        .remove::<PlannedStructureProgress>();
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
/// - [`PlannedKind::ProductionFacility`] completes into a
///   [`ProductionFacility`] carrying `first_target` as its
///   first `current_target`, plus a local [`Stockpile`]
///   buffer so the production chain can pull minerals
///   through the facility's own staging buffer (matching the
///   seed facility shape in the default scenario). The
///   `OwnerSwarm` is preserved through the promotion, so
///   the completed facility keeps the swarm that painted
///   the Build Zone the plan lived in. `first_target` is
///   `Some(target)` for plans created by the
///   `PlannedProductionTarget` sidecar, or `None` for test
///   fixtures that bypass the auto-creation system (the
///   same fallback the pre-multi-swarm tests rely on).
/// - [`PlannedKind::Charger`] completes into a
///   [`crate::nanobot::Charger`] built from the plan's
///   cell, with the default capacity / radius / initial
///   amount so the existing charge sustain loop sees a
///   "ready to serve" charger with `AUTO_CHARGER_INITIAL_AMOUNT`
///   material already on hand. The `OwnerSwarm` is
///   preserved so the completed charger keeps the swarm
///   that painted the Defend cell the plan lived in.
///   `first_target` is unused for this kind; the
///   pre-existing test fixtures that pre-spawn a Charger
///   already establish the default-shape contract.
fn promote_planned_to_completion(
    commands: &mut Commands,
    planned_entity: Entity,
    kind: PlannedKind,
    world_pos: Vec2,
    cell: IVec2,
    first_target: Option<NanobotType>,
    structure_sprites: &StructureSprites,
) {
    let visual = completed_visual_bundle(kind, structure_sprites, world_pos);
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
            // The first production target was decided when
            // the plan was created. The work system reads
            // it off the sidecar and passes it in, so the
            // promotion path is a single function call
            // with no `Entity` lookup.
            let mut facility = ProductionFacility::new();
            facility.current_target = first_target;
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
            // Promote to a real `Charger` with the default
            // shape (`AUTO_CHARGER_INITIAL_AMOUNT` material
            // already on hand, full capacity, default
            // radius). The `OwnerSwarm` stays on the entity
            // through Bevy's component-merge semantics, so
            // the completed charger keeps the swarm that
            // painted the Defend cell the plan lived in.
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
        Transform::from_translation(world_pos.extend(GAMEPLAY_SPRITE_Z)),
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
    world_pos: Vec2,
) -> (Sprite, Transform, StructureVisual) {
    let mut sprite = structure_sprites.sprite(kind, StructureVisualState::Completed);
    sprite.color = completed_visual_color();
    sprite.custom_size = Some(Vec2::splat(PLANNED_STRUCTURE_FOOTPRINT));
    (
        sprite,
        Transform::from_translation(world_pos.extend(GAMEPLAY_SPRITE_Z)),
        StructureVisual::completed(kind),
    )
}

/// Empty mineral buffer used by every completed planned
/// kind that needs a local `Stockpile` (Sink Stockpile,
/// Production Facility). The `Source` kind is satisfied by
/// the same buffer shape. Kept as a single source of truth
/// so future planned kinds (Charger) can drop in without
/// re-stating the literal.
fn empty_mineral_stockpile() -> Stockpile {
    Stockpile {
        kind: ResourceKind::Minerals,
        amount: 0,
        capacity: 1000,
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
        app.add_systems(
            Update,
            (
                sink_stockpile_demand_system,
                worker_planned_structure_claim_system,
                worker_planned_structure_arrive_system,
                worker_planned_structure_work_system,
            )
                .chain()
                .after(crate::nanobot::move_velocity_system),
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
