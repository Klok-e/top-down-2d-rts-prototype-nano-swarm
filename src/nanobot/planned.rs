//! Automatic support-structure demand, authored kinds, and initial visuals.
//!
//! Construction ownership, progress, access validation, evacuation, and activation
//! are implemented by [`super::structure_lifecycle`]. Demand producers create plans;
//! they do not mutate lifecycle progress or complete structures.

use crate::navigation::Obstacle;
use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use super::structure_lifecycle::*;

use crate::GAMEPLAY_SPRITE_Z;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::SupportCondition;
use crate::nanobot::components::{Swarm, SwarmId};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::production::{OwnerSwarm, ProductionFacility};
use crate::resources::{ResourceDeposit, SinkUpkeep, Stockpile, StockpileRole};
use crate::structure_sprites::{StructureSprites, StructureVisual, StructureVisualState};

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

    /// Whole cells along each side of this kind's Building Footprint, shared by
    /// planned, built, and authored starting structures.
    pub const fn footprint_cells(self) -> u32 {
        match self {
            PlannedKind::ProductionFacility => 3,
            PlannedKind::SourceStockpile | PlannedKind::SinkStockpile | PlannedKind::Charger => 1,
        }
    }

    /// Half the side length of this kind's Building Footprint in world units.
    pub const fn footprint_half_extent(self) -> f32 {
        self.footprint_cells() as f32 * crate::navigation::CELL_WIDTH / 2.0
    }

    /// The aligned whole-cell transform of this kind's footprint near `position`.
    pub fn footprint_transform(self, position: Vec3) -> Transform {
        let scale = 2.0 * self.footprint_half_extent() / crate::navigation::STRUCTURE_SPRITE_SIZE;
        crate::navigation::align_structure(
            Transform::from_translation(position).with_scale(Vec3::new(scale, scale, 1.0)),
        )
    }
}

/// Tint for the translucent artwork on Planned Structures.
pub const fn planned_visual_color() -> Color {
    Color::srgba(1.0, 1.0, 1.0, 0.85)
}

/// Neutral tint preserves the artwork palette and leaves headroom for hit flashes.
pub const fn completed_visual_color() -> Color {
    Color::srgb(0.86, 0.86, 0.86)
}

/// Build-painted cells where a consumer may place its local Sink Stockpile.
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

/// A pending or completed Production Facility that may ask for a Sink.
struct SinkDemandSite {
    cell: IVec2,
    owner: Entity,
    facility: Entity,
    transform: Transform,
}

/// Plan Sink Stockpiles only where one would repay its Maintenance, and record
/// on every Sink whether it still does. Raw Build paint is only a placement
/// constraint: it does not create construction demand by itself. A pending or
/// completed Production Facility in a Build cell may get one Sink in its
/// same-owner local 3x3 Build zone, at the valid spot nearest the facility,
/// without overlapping deposits or other support structures. Terminals also
/// draw directly from Source Stockpiles, so the Sink is planned only when its
/// delivery path to the facility beats the nearest own Source's by
/// [`super::sink_break_even_margin`]. A Sink pays while it does so for at least
/// one own facility whose zone contains it; one that pays for none gets no
/// Maintenance or new material and decays. A Planned Sink is still built.
/// Chargers accept direct delivery and do not create Sink demand. Runs on the
/// allocation cadence because it computes walking paths.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sink_stockpile_demand_system(
    mut commands: Commands,
    access: super::construction_access::ConstructionAccess,
    mut payoff: super::SinkPayoff,
    grid: Res<IntentGrid>,
    structure_sprites: Res<StructureSprites>,
    tuning: Res<crate::tuning::Tuning>,
    planned: Query<(Entity, &PlannedStructure, &Transform, Option<&OwnerSwarm>)>,
    mut stockpiles: Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
        Option<&mut SinkUpkeep>,
    )>,
    facilities: Query<(Entity, &Transform, Option<&OwnerSwarm>), With<ProductionFacility>>,
    chargers: Query<(&Transform, Option<&OwnerSwarm>), With<crate::nanobot::Charger>>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    let mut access_layout = access.snapshot();
    let margin = super::sink_break_even_margin(&tuning);
    let swarm_by_id: HashMap<SwarmId, Entity> = swarms.iter().map(|(e, id)| (*id, e)).collect();
    let mut obstacles: Vec<Obstacle> = deposits
        .iter()
        .map(|(deposit, transform)| {
            Obstacle::deposit(transform.translation.truncate(), deposit.radius)
        })
        .collect();
    for (_, _, transform, ..) in &stockpiles {
        obstacles.push(Obstacle::structure(transform));
    }
    // Planned Structures of any kind are in the obstacle
    // list so a fresh Sink Stockpile cannot overlap a
    // pending Production Facility or Charger plan. Only
    // Production Facility plans create sink-side demand:
    // chargers are direct-delivery terminals and do not
    // auto-plan Sink Stockpiles (ADR-0005).
    let mut demand_sites: Vec<SinkDemandSite> = Vec::new();
    for (entity, planned_structure, transform, owner) in &planned {
        obstacles.push(Obstacle::structure(transform));
        if planned_structure.kind == PlannedKind::ProductionFacility
            && let Some(owner) = owner
        {
            demand_sites.push(SinkDemandSite {
                cell: world_to_cell(transform.translation.truncate()),
                owner: owner.0,
                facility: entity,
                transform: *transform,
            });
        }
    }
    for (entity, transform, owner) in &facilities {
        obstacles.push(Obstacle::structure(transform));
        if let Some(owner) = owner {
            demand_sites.push(SinkDemandSite {
                cell: world_to_cell(transform.translation.truncate()),
                owner: owner.0,
                facility: entity,
                transform: *transform,
            });
        }
    }
    // Chargers are direct-delivery terminals fed by haulers;
    // they deliberately do not create Sink Stockpile demand.
    // They stay in the obstacle list above so facility-side
    // sink plans cannot overlap them.
    for (transform, _) in &chargers {
        obstacles.push(Obstacle::structure(transform));
    }
    demand_sites.sort_by_key(|site| (site.cell.x, site.cell.y, site.facility));

    let mut paying = HashSet::new();
    let mut newly_planned: Vec<Vec2> = Vec::new();
    for site in demand_sites {
        let Ok((_, placement_swarm)) = swarms.get(site.owner) else {
            continue;
        };
        let placement_swarm = *placement_swarm;
        let owned = |candidate: Option<&OwnerSwarm>| match candidate {
            Some(owner) => swarms
                .get(owner.0)
                .is_ok_and(|(_, id)| *id == placement_swarm),
            None => false,
        };
        let zone_cells = sink_stockpile_zone_cells(&grid, site.cell, site.owner, &swarm_by_id);
        if zone_cells.is_empty() {
            continue;
        }
        let in_zone = |position: Vec2| zone_cells.contains(&world_to_cell(position));
        let mut path_to_facility = |from: &Transform| {
            payoff.delivery_path(&grid, placement_swarm, from, site.facility, &site.transform)
        };
        // Only a Source terminals can draw from competes with a Sink.
        let nearest_source = stockpiles
            .iter()
            .filter(|(_, _, _, role, owner, condition, _)| {
                !matches!(role, Some(StockpileRole::Sink))
                    && owned(*owner)
                    && condition.is_none_or(|condition| condition.is_operational())
            })
            .filter_map(|(_, _, transform, ..)| path_to_facility(transform))
            .fold(f32::INFINITY, f32::min);
        let pays = |path: Option<f32>| path.is_some_and(|path| path + margin < nearest_source);

        let mut served = false;
        for (entity, _, transform, role, owner, ..) in &stockpiles {
            if matches!(role, Some(StockpileRole::Sink))
                && owned(owner)
                && in_zone(transform.translation.truncate())
                && pays(path_to_facility(transform))
            {
                paying.insert(entity);
                served = true;
            }
        }
        let plan_pending = planned.iter().any(|(_, plan, transform, owner)| {
            plan.kind == PlannedKind::SinkStockpile
                && in_zone(transform.translation.truncate())
                && owned(owner)
        }) || newly_planned.iter().any(|position| in_zone(*position));
        if served || plan_pending {
            continue;
        }

        let mut local_obstacles = obstacles.clone();
        local_obstacles.extend(newly_planned.iter().map(|pos| Obstacle::planned(*pos)));
        let mut sorted_cells = zone_cells.clone();
        sorted_cells.sort_unstable_by_key(|cell| (cell.x, cell.y));
        // Sites come nearest-first. Payoff is checked before access, from
        // cached paths: a farther site saves even less, so the first reachable
        // site that does not pay ends the search, and so does a Pending access
        // answer.
        let Some((placement_cell, placement_pos)) =
            crate::nanobot::placement::SiteLattice::build(&sorted_cells, &local_obstacles)
                .find_nearest(site.transform.translation.truncate(), |position| {
                    let spot = PlannedKind::SinkStockpile.footprint_transform(position.extend(0.0));
                    let Some(path) = path_to_facility(&spot) else {
                        return Some(false);
                    };
                    if !pays(Some(path)) {
                        return None;
                    }
                    (!access_layout.is_pending()).then(|| {
                        access.accepts(
                            &access_layout,
                            &grid,
                            placement_swarm,
                            PlannedKind::SinkStockpile,
                            position,
                        )
                    })
                })
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
            PlannedStructure::new(PlannedKind::SinkStockpile, placement_cell)
                .with_work_budget(tuning.construction.work_ticks),
            planned_visual_components(
                PlannedKind::SinkStockpile,
                &structure_sprites,
                placement_pos,
            ),
        ));
        entity_commands.insert(OwnerSwarm(site.owner));
    }

    for (entity, _, _, role, _, _, upkeep) in &mut stockpiles {
        if !matches!(role, Some(StockpileRole::Sink)) {
            continue;
        }
        let upkeep_now = SinkUpkeep {
            pays: paying.contains(&entity),
        };
        match upkeep {
            Some(mut upkeep) => {
                upkeep.set_if_neq(upkeep_now);
            }
            None => {
                commands.entity(entity).insert(upkeep_now);
            }
        }
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
    StructureVisual::planned(kind).components(
        structure_sprites.handle(kind, StructureVisualState::Planned),
        kind.footprint_transform(world_pos.extend(GAMEPLAY_SPRITE_Z)),
    )
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
        assert_eq!(
            p.available_work(),
            crate::tuning::Tuning::default().construction.work_ticks
        );
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
    fn default_work_budget_matches_shipped_construction_duration() {
        // The default is the shipped automatic-construction duration.
        {
            assert!(crate::tuning::Tuning::default().construction.work_ticks > 0)
        };
        {
            assert!(crate::tuning::Tuning::default().construction.work_ticks <= 1_500)
        };
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
