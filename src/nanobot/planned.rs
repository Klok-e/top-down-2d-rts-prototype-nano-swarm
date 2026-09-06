//! Automatic support-structure demand, authored kinds, and initial visuals.
//!
//! Construction ownership, progress, access validation, evacuation, and activation
//! are implemented by [`super::structure_lifecycle`]. Demand producers create plans;
//! they do not mutate lifecycle progress or complete structures.

use crate::navigation::Obstacle;
use std::collections::HashMap;

use bevy::prelude::*;

use super::structure_lifecycle::*;

use crate::GAMEPLAY_SPRITE_Z;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::components::{Swarm, SwarmId};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::production::{OwnerSwarm, ProductionFacility};
use crate::resources::{ResourceDeposit, Stockpile, StockpileRole};
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
/// All variants are data-less so [`PlannedKind::ALL`] can stay a `const` array.
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
    /// existing capacity is too busy. The completed facility
    /// selects from current Population Demand after funding.
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

/// Build-painted cells where a consumer may place its local Sink Stockpile.
/// Both planning and collapse recovery use this helper so ownership and
/// consumer-local topology cannot diverge.
pub(crate) fn sink_stockpile_zone_cells(
    grid: &IntentGrid,
    consumer_cell: IVec2,
    consumer_owner: Entity,
    swarm_by_id: &HashMap<SwarmId, Entity>,
) -> Vec<IVec2> {
    let Some((&swarm, _)) = swarm_by_id
        .iter()
        .find(|(_, entity)| **entity == consumer_owner)
    else {
        return Vec::new();
    };
    if !grid
        .cell(consumer_cell)
        .is_some_and(|intent| intent.has_owned(IntentKind::Build, swarm))
    {
        return Vec::new();
    }
    let mut zone_cells = Vec::new();
    for dx in -1..=1 {
        for dy in -1..=1 {
            let cell = consumer_cell + IVec2::new(dx, dy);
            if grid
                .cell(cell)
                .is_some_and(|intent| intent.has_owned(IntentKind::Build, swarm))
            {
                zone_cells.push(cell);
            }
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
    let mut demand_sites: Vec<(IVec2, Entity)> = Vec::new();
    for (planned_structure, transform, owner) in &planned {
        obstacles.push(Obstacle::structure(transform));
        if planned_structure.kind == PlannedKind::ProductionFacility
            && let Some(owner) = owner
        {
            demand_sites.push((world_to_cell(transform.translation.truncate()), owner.0));
        }
    }
    for (transform, _) in &facilities {
        obstacles.push(Obstacle::structure(transform));
    }
    for (transform, _) in &chargers {
        obstacles.push(Obstacle::structure(transform));
    }

    for (transform, owner) in &facilities {
        if let Some(owner) = owner {
            demand_sites.push((world_to_cell(transform.translation.truncate()), owner.0));
        }
    }
    // Chargers are direct-delivery terminals fed by haulers;
    // they deliberately do not create Sink Stockpile demand.
    // They stay in the obstacle list above so facility-side
    // sink plans cannot overlap them.
    demand_sites.sort_by_key(|(cell, _)| (cell.x, cell.y));
    demand_sites.dedup();

    let mut newly_planned: Vec<Vec2> = Vec::new();
    for (cell, owner) in demand_sites {
        let Ok((_, placement_swarm)) = swarms.get(owner) else {
            continue;
        };
        let placement_swarm = *placement_swarm;
        let sink_owner_matches = |candidate: Option<&OwnerSwarm>| match candidate {
            Some(owner) => swarms
                .get(owner.0)
                .is_ok_and(|(_, id)| *id == placement_swarm),
            None => false,
        };
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
                    && sink_owner_matches(stockpile_owner)
            })
            || planned
                .iter()
                .any(|(planned_structure, transform, plan_owner)| {
                    planned_structure.kind == PlannedKind::SinkStockpile
                        && in_zone(world_to_cell(transform.translation.truncate()))
                        && sink_owner_matches(plan_owner)
                });
        if sink_exists {
            continue;
        }
        let mut local_obstacles = obstacles.clone();
        local_obstacles.extend(newly_planned.iter().map(|pos| Obstacle::planned(*pos)));
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
        entity_commands.insert(OwnerSwarm(owner));
    }
}

/// The "build pending" visual shared by every planned kind.
/// Each auto-creation path pairs the [`PlannedStructure`]
/// component with this bundle, then completes by
/// the lifecycle completion visual on promotion. Bevy replaces
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

/// Default capacity for completed Source and Sink Stockpiles.
/// One full hauler load is one tenth of this buffer.
pub const DEFAULT_STOCKPILE_CAPACITY: u32 = 200;

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
        assert_eq!(p.available_work(), DEFAULT_PLANNED_WORK_TICKS);
        assert!(p.can_accept_worker());
        assert!(!p.is_complete());
    }

    #[test]
    fn planned_structure_reports_unclaimed_only_when_no_worker() {
        let mut p = PlannedStructure::new(PlannedKind::SourceStockpile, IVec2::new(1, 1));
        assert!(p.can_accept_worker());
        // The reservation type is a plain `Option<Entity>`; the
        // test uses a dummy entity handle since the field's
        // contract is "is there a worker?", not "is the worker
        // still alive?".
        assert!(p.try_claim(Entity::PLACEHOLDER));
        assert!(!p.can_accept_worker());
    }

    #[test]
    fn planned_structure_completes_only_when_budget_zero() {
        let mut p = PlannedStructure::new(PlannedKind::SourceStockpile, IVec2::new(0, 0));
        p = p.with_work_remaining(1);
        assert!(!p.is_complete());
        p = p.with_work_remaining(0);
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
