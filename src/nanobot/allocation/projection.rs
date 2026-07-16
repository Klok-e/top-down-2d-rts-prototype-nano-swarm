//! Dirty-region projection from authoritative Intent/ECS state.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use super::{
    ALLOCATION_REGION_CELLS, ActionableOpportunity, AllocationRegion, OpportunityCategory,
    OpportunityTarget,
};
use crate::ZONE_BLOCK_SIZE;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::{
    Charger, DefendPressure, OwnerSwarm, PlannedStructure, ProductionFacility, Structure,
    SupportCondition, SwarmId, cell_overlaps_circle,
};
use crate::resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole};

/// Region-indexed derived work. Callers may invalidate cells/regions without
/// owning projection logic; next projection pass replaces those regions only.
#[derive(Debug, Default, Resource)]
pub struct ActionableProjection {
    by_region: BTreeMap<AllocationRegion, Vec<ActionableOpportunity>>,
    dirty_regions: BTreeSet<AllocationRegion>,
}

impl ActionableProjection {
    pub fn invalidate_cell(&mut self, cell: IVec2) {
        self.invalidate_region(AllocationRegion::for_cell(cell));
    }

    pub fn invalidate_region(&mut self, region: AllocationRegion) {
        self.dirty_regions.insert(region);
    }

    pub fn dirty_region_count(&self) -> usize {
        self.dirty_regions.len()
    }

    pub fn opportunities(&self, region: AllocationRegion) -> &[ActionableOpportunity] {
        self.by_region.get(&region).map_or(&[], Vec::as_slice)
    }

    pub fn iter_regions(
        &self,
    ) -> impl Iterator<Item = (AllocationRegion, &[ActionableOpportunity])> {
        self.by_region
            .iter()
            .map(|(region, opportunities)| (*region, opportunities.as_slice()))
    }

    fn take_dirty_regions(&mut self) -> Vec<AllocationRegion> {
        std::mem::take(&mut self.dirty_regions)
            .into_iter()
            .collect()
    }
}

#[derive(Clone, Copy)]
struct StockpileSnapshot {
    entity: Entity,
    cell: IVec2,
    kind: ResourceKind,
    role: StockpileRole,
    amount: u32,
    free_space: u32,
    owner: Option<SwarmId>,
}

#[derive(Clone, Copy)]
struct SinkSnapshot {
    entity: Entity,
    kind: ResourceKind,
    free_space: u32,
    owner: Option<SwarmId>,
    source_role: SourceRole,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceRole {
    Source,
    Sink,
}

/// Consume independent projection dirtiness, include changed ECS work, then
/// replace only affected allocation regions. This system is intentionally not
/// registered by production plugins during foundation phase.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn project_actionable_opportunities_system(
    mut grid: ResMut<IntentGrid>,
    mut projection: ResMut<ActionableProjection>,
    pressure: Option<Res<DefendPressure>>,
    deposits: Query<(
        Entity,
        Ref<ResourceDeposit>,
        Ref<Transform>,
        Option<Ref<OwnerSwarm>>,
    )>,
    mut removed_deposit_owners: RemovedComponents<OwnerSwarm>,
    planned: Query<(Entity, Ref<PlannedStructure>, Option<&OwnerSwarm>)>,
    structures: Query<(Entity, Ref<Structure>, Ref<Transform>, Option<&OwnerSwarm>)>,
    stockpiles: Query<(
        Entity,
        Ref<Stockpile>,
        Ref<Transform>,
        Option<Ref<StockpileRole>>,
        Option<Ref<OwnerSwarm>>,
        Option<Ref<SupportCondition>>,
    )>,
    facilities: Query<(
        Entity,
        Ref<ProductionFacility>,
        Option<Ref<OwnerSwarm>>,
        Option<Ref<SupportCondition>>,
    )>,
    chargers: Query<(
        Entity,
        Ref<Charger>,
        Option<Ref<OwnerSwarm>>,
        Option<Ref<SupportCondition>>,
    )>,
    swarms: Query<&SwarmId>,
    entities: Query<Entity>,
) {
    if pressure
        .as_ref()
        .is_some_and(|pressure| pressure.is_changed())
    {
        for (cell, _intent) in grid
            .iter_active_cells()
            .filter(|(_, intent)| intent.has(IntentKind::Defend))
        {
            projection.invalidate_cell(cell);
        }
    }
    for cell in grid.drain_projection_dirty() {
        projection.invalidate_cell(cell);
        for (_, deposit, transform, _) in &deposits {
            if cell_overlaps_circle(cell, transform.translation.truncate(), deposit.radius) {
                invalidate_circle_regions(
                    &mut projection,
                    transform.translation.truncate(),
                    deposit.radius,
                );
            }
        }
    }

    let stale_regions = projection
        .iter_regions()
        .filter_map(|(region, opportunities)| {
            opportunities
                .iter()
                .any(|opportunity| match opportunity.target {
                    OpportunityTarget::Gather { deposit, .. } => match deposits.get(deposit) {
                        Err(_) => true,
                        Ok((_, deposit, transform, owner)) => {
                            deposit.is_changed()
                                || transform.is_changed()
                                || owner.as_ref().is_some_and(|owner| owner.is_changed())
                                || deposit.amount == 0
                        }
                    },
                    OpportunityTarget::PlannedBuild { structure, .. } => {
                        planned.get(structure).is_err()
                    }
                    OpportunityTarget::Maintenance { structure } => {
                        structures.get(structure).is_err()
                    }
                    OpportunityTarget::Defend { .. } => false,
                    OpportunityTarget::Haul { source, sink, .. } => {
                        entities.get(source).is_err() || entities.get(sink).is_err()
                    }
                })
                .then_some(region)
        })
        .collect::<Vec<_>>();
    for region in stale_regions {
        projection.invalidate_region(region);
    }

    for (_, deposit, transform, owner) in &deposits {
        if deposit.is_changed()
            || transform.is_changed()
            || owner.as_ref().is_some_and(|owner| owner.is_changed())
        {
            invalidate_circle_regions(
                &mut projection,
                transform.translation.truncate(),
                deposit.radius,
            );
        }
    }
    for entity in removed_deposit_owners.read() {
        if let Ok((_, deposit, transform, _)) = deposits.get(entity) {
            invalidate_circle_regions(
                &mut projection,
                transform.translation.truncate(),
                deposit.radius,
            );
        }
    }
    for (_, planned, _) in &planned {
        if planned.is_changed() {
            projection.invalidate_cell(planned.cell);
        }
    }
    for (_, structure, transform, _) in &structures {
        if structure.is_changed() || transform.is_changed() {
            projection.invalidate_cell(crate::nanobot::world_to_cell(
                transform.translation.truncate(),
            ));
        }
    }

    let mut haul_sinks_changed = false;
    for (_, stockpile, transform, role, owner, condition) in &stockpiles {
        if stockpile.is_changed()
            || transform.is_changed()
            || role.as_ref().is_some_and(|role| role.is_changed())
            || owner.as_ref().is_some_and(|owner| owner.is_changed())
            || condition
                .as_ref()
                .is_some_and(|condition| condition.is_changed())
        {
            projection.invalidate_cell(crate::nanobot::world_to_cell(
                transform.translation.truncate(),
            ));
            haul_sinks_changed = true;
        }
    }
    haul_sinks_changed |= facilities.iter().any(|(_, facility, owner, condition)| {
        facility.is_changed()
            || owner.as_ref().is_some_and(|owner| owner.is_changed())
            || condition
                .as_ref()
                .is_some_and(|condition| condition.is_changed())
    });
    haul_sinks_changed |= chargers.iter().any(|(_, charger, owner, condition)| {
        charger.is_changed()
            || owner.as_ref().is_some_and(|owner| owner.is_changed())
            || condition
                .as_ref()
                .is_some_and(|condition| condition.is_changed())
    });
    if haul_sinks_changed {
        for (_, stockpile, transform, _, _, _) in &stockpiles {
            if stockpile.amount > 0 {
                projection.invalidate_cell(crate::nanobot::world_to_cell(
                    transform.translation.truncate(),
                ));
            }
        }
    }

    let stockpile_snapshots = stockpiles
        .iter()
        .filter_map(|(entity, stockpile, transform, role, owner, condition)| {
            if condition.is_some_and(|condition| !condition.is_operational()) {
                return None;
            }
            Some(StockpileSnapshot {
                entity,
                cell: crate::nanobot::world_to_cell(transform.translation.truncate()),
                kind: stockpile.kind,
                role: role.as_deref().copied().unwrap_or_default(),
                amount: stockpile.amount,
                free_space: stockpile.free_space(),
                owner: resolve_owner(owner.as_deref(), &swarms)?,
            })
        })
        .collect::<Vec<_>>();
    let mut sinks = stockpile_snapshots
        .iter()
        .filter(|stockpile| stockpile.role == StockpileRole::Sink)
        .map(|stockpile| SinkSnapshot {
            entity: stockpile.entity,
            kind: stockpile.kind,
            free_space: stockpile.free_space,
            owner: stockpile.owner,
            source_role: SourceRole::Source,
        })
        .collect::<Vec<_>>();
    sinks.extend(
        facilities
            .iter()
            .filter_map(|(entity, facility, owner, condition)| {
                if condition.is_some_and(|condition| !condition.is_operational()) {
                    return None;
                }
                Some(SinkSnapshot {
                    entity,
                    kind: facility.input_kind,
                    free_space: facility.input_free_space(),
                    owner: resolve_owner(owner.as_deref(), &swarms)?,
                    source_role: SourceRole::Sink,
                })
            }),
    );
    sinks.extend(
        chargers
            .iter()
            .filter_map(|(entity, charger, owner, condition)| {
                if condition.is_some_and(|condition| !condition.is_operational()) {
                    return None;
                }
                Some(SinkSnapshot {
                    entity,
                    kind: charger.kind,
                    free_space: charger.free_space(),
                    owner: resolve_owner(owner.as_deref(), &swarms)?,
                    source_role: SourceRole::Sink,
                })
            }),
    );

    let dirty_regions = projection.take_dirty_regions();
    for region in dirty_regions {
        let mut opportunities = Vec::new();
        project_intent_work(
            region,
            &grid,
            &deposits,
            &structures,
            &swarms,
            pressure.as_deref(),
            &mut opportunities,
        );
        project_planned_work(region, &planned, &swarms, &mut opportunities);
        project_haul_work(region, &stockpile_snapshots, &sinks, &mut opportunities);
        opportunities.sort_by_key(opportunity_sort_key);
        projection.by_region.insert(region, opportunities);
    }
}

#[allow(clippy::type_complexity)]
fn project_intent_work(
    region: AllocationRegion,
    grid: &IntentGrid,
    deposits: &Query<(
        Entity,
        Ref<ResourceDeposit>,
        Ref<Transform>,
        Option<Ref<OwnerSwarm>>,
    )>,
    structures: &Query<(Entity, Ref<Structure>, Ref<Transform>, Option<&OwnerSwarm>)>,
    swarms: &Query<&SwarmId>,
    pressure: Option<&DefendPressure>,
    out: &mut Vec<ActionableOpportunity>,
) {
    for (entity, deposit, transform, owner) in deposits.iter() {
        let Some(deposit_owner) = resolve_owner(owner.as_deref(), swarms) else {
            continue;
        };
        if deposit.amount == 0 {
            continue;
        }

        let mut anchors = Vec::<(Option<SwarmId>, IVec2)>::new();
        for (cell, intent) in grid.iter_active_cells().filter(|(cell, intent)| {
            intent.has(IntentKind::Gather)
                && owners_compatible(intent.owner(IntentKind::Gather), deposit_owner)
                && cell_overlaps_circle(*cell, transform.translation.truncate(), deposit.radius)
        }) {
            let effective_owner = deposit_owner.or(intent.owner(IntentKind::Gather));
            if let Some((_, anchor)) = anchors
                .iter_mut()
                .find(|(owner, _)| *owner == effective_owner)
            {
                if (cell.x, cell.y) < (anchor.x, anchor.y) {
                    *anchor = cell;
                }
            } else {
                anchors.push((effective_owner, cell));
            }
        }

        if deposit_owner.is_none() && anchors.iter().any(|(owner, _)| owner.is_none()) {
            anchors.retain(|(owner, _)| owner.is_none());
        }
        anchors.sort_by_key(|(owner, _)| owner.map_or((false, 0), |owner| (true, owner.0)));

        for (owner, cell) in anchors {
            if AllocationRegion::for_cell(cell) == region {
                out.push(ActionableOpportunity {
                    region,
                    category: OpportunityCategory::Gather,
                    target: OpportunityTarget::Gather {
                        deposit: entity,
                        cell,
                    },
                    cell,
                    owner,
                    available_work: deposit.amount,
                });
            }
        }
    }

    for (entity, structure, transform, owner) in structures.iter() {
        if !structure.needs_maintenance() {
            continue;
        }
        let cell = crate::nanobot::world_to_cell(transform.translation.truncate());
        if AllocationRegion::for_cell(cell) != region {
            continue;
        }
        let Some(owner) = resolve_owner(owner, swarms) else {
            continue;
        };
        out.push(ActionableOpportunity {
            region,
            category: OpportunityCategory::Maintenance,
            target: OpportunityTarget::Maintenance { structure: entity },
            cell,
            owner,
            available_work: 1,
        });
    }

    let min = region.min_cell();
    for dy in 0..ALLOCATION_REGION_CELLS {
        for dx in 0..ALLOCATION_REGION_CELLS {
            let cell = min + IVec2::new(dx, dy);
            let Some(intent) = grid.cell(cell) else {
                continue;
            };

            if intent.has(IntentKind::Defend) {
                let owner = intent.owner(IntentKind::Defend);
                out.push(ActionableOpportunity {
                    region,
                    category: OpportunityCategory::Defend,
                    target: OpportunityTarget::Defend { cell },
                    cell,
                    owner,
                    available_work: owner
                        .and_then(|owner| {
                            pressure.map(|pressure| pressure.get_for(owner, cell).ceil() as u32)
                        })
                        .unwrap_or(1)
                        .max(1),
                });
            }
        }
    }
}

fn project_planned_work(
    region: AllocationRegion,
    planned: &Query<(Entity, Ref<PlannedStructure>, Option<&OwnerSwarm>)>,
    swarms: &Query<&SwarmId>,
    out: &mut Vec<ActionableOpportunity>,
) {
    for (entity, planned, owner) in planned.iter() {
        if AllocationRegion::for_cell(planned.cell) != region || planned.work_remaining == 0 {
            continue;
        }
        let Some(owner) = resolve_owner(owner, swarms) else {
            continue;
        };
        out.push(ActionableOpportunity {
            region,
            category: OpportunityCategory::PlannedBuild,
            target: OpportunityTarget::PlannedBuild {
                structure: entity,
                kind: planned.kind,
            },
            cell: planned.cell,
            owner,
            available_work: planned.work_remaining,
        });
    }
}

fn project_haul_work(
    region: AllocationRegion,
    stockpiles: &[StockpileSnapshot],
    sinks: &[SinkSnapshot],
    out: &mut Vec<ActionableOpportunity>,
) {
    for source in stockpiles {
        if AllocationRegion::for_cell(source.cell) != region || source.amount == 0 {
            continue;
        }
        for sink in sinks {
            if source.entity == sink.entity
                || source.kind != sink.kind
                || sink.free_space == 0
                || !source_role_matches(source.role, sink.source_role)
                || source.owner.is_none()
                || source.owner != sink.owner
            {
                continue;
            }
            out.push(ActionableOpportunity {
                region,
                category: OpportunityCategory::Haul,
                target: OpportunityTarget::Haul {
                    source: source.entity,
                    sink: sink.entity,
                    kind: source.kind,
                },
                cell: source.cell,
                owner: source.owner,
                available_work: source.amount.min(sink.free_space),
            });
        }
    }
}

fn invalidate_circle_regions(projection: &mut ActionableProjection, center: Vec2, radius: f32) {
    let radius = radius.max(0.0);
    let lower = center - Vec2::splat(radius);
    let mut min = crate::nanobot::world_to_cell(lower);
    if (lower.x / ZONE_BLOCK_SIZE).fract() == 0.0 {
        min.x -= 1;
    }
    if (lower.y / ZONE_BLOCK_SIZE).fract() == 0.0 {
        min.y -= 1;
    }
    let max = crate::nanobot::world_to_cell(center + Vec2::splat(radius));
    for y in min.y..=max.y {
        for x in min.x..=max.x {
            projection.invalidate_cell(IVec2::new(x, y));
        }
    }
}

fn resolve_owner(owner: Option<&OwnerSwarm>, swarms: &Query<&SwarmId>) -> Option<Option<SwarmId>> {
    match owner {
        None => Some(None),
        Some(owner) => swarms.get(owner.0).ok().copied().map(Some),
    }
}

fn owners_compatible(left: Option<SwarmId>, right: Option<SwarmId>) -> bool {
    left.is_none() || right.is_none() || left == right
}

fn source_role_matches(role: StockpileRole, required: SourceRole) -> bool {
    match required {
        SourceRole::Source => role == StockpileRole::Source,
        SourceRole::Sink => role == StockpileRole::Sink,
    }
}

fn opportunity_sort_key(opportunity: &ActionableOpportunity) -> (u8, u8, u64, u64, i32, i32) {
    let category = match opportunity.category {
        OpportunityCategory::PlannedBuild => 0,
        OpportunityCategory::Maintenance => 1,
        OpportunityCategory::Gather => 2,
        OpportunityCategory::Defend => 3,
        OpportunityCategory::Haul => 4,
    };
    let (target_kind, first, second) = match opportunity.target {
        OpportunityTarget::Gather { deposit, .. } => (0, deposit.to_bits(), 0),
        OpportunityTarget::PlannedBuild { structure, .. } => (1, structure.to_bits(), 0),
        OpportunityTarget::Maintenance { structure } => (2, structure.to_bits(), 0),
        OpportunityTarget::Defend { .. } => (3, 0, 0),
        OpportunityTarget::Haul { source, sink, .. } => (4, source.to_bits(), sink.to_bits()),
    };
    (
        category,
        target_kind,
        first,
        second,
        opportunity.cell.y,
        opportunity.cell.x,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_invalidation_includes_cells_touching_exact_lower_boundary() {
        let mut projection = ActionableProjection::default();
        let center = Vec2::new(9.0 * ZONE_BLOCK_SIZE, 0.5 * ZONE_BLOCK_SIZE);

        invalidate_circle_regions(&mut projection, center, ZONE_BLOCK_SIZE);

        assert!(
            projection
                .dirty_regions
                .contains(&AllocationRegion::for_cell(IVec2::new(7, 0)))
        );
    }
}
