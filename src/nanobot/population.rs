//! Workload-derived total population demand.

use std::collections::{HashMap, HashSet};

use super::work_access::{WorkAccess, WorkReachability};
use bevy::prelude::*;

use crate::nanobot::{
    ActionableProjection, HAULER_CARRY_CAPACITY, NanobotType, OpportunityCategory,
    OpportunityTarget, RegionalAllocationSet, SwarmId, TerritorySnapshot,
    defender_population_demand, production_facility_pick_target_system,
};

/// Desired population by swarm and Nanobot Type, derived from discrete useful
/// work capacity.
#[derive(Debug, Default, Resource)]
pub struct PopulationDemand {
    desired: HashMap<(SwarmId, NanobotType), u32>,
}

impl PopulationDemand {
    pub fn desired_for(&self, swarm: SwarmId, kind: NanobotType) -> u32 {
        self.desired
            .get(&(swarm, kind))
            .copied()
            .unwrap_or_default()
    }

    pub fn total_for(&self, swarm: SwarmId) -> u32 {
        NanobotType::ALL
            .iter()
            .map(|kind| self.desired_for(swarm, *kind))
            .sum()
    }

    pub fn has_shortage(&self, swarm: SwarmId, counts: &HashMap<NanobotType, u32>) -> bool {
        NanobotType::ALL.iter().any(|kind| {
            counts.get(kind).copied().unwrap_or_default() < self.desired_for(swarm, *kind)
        })
    }

    /// Return the type with the greatest shortage relative to its own demand.
    /// Equal ratios prefer the larger missing count, then stable type order.
    pub(crate) fn most_underfilled_type(
        &self,
        swarm: SwarmId,
        covered: &HashMap<NanobotType, u32>,
    ) -> Option<NanobotType> {
        let mut best: Option<(u32, u32, NanobotType)> = None;
        for kind in NanobotType::ALL {
            let desired = self.desired_for(swarm, kind);
            let available = covered.get(&kind).copied().unwrap_or_default();
            let missing = desired.saturating_sub(available);
            if missing == 0 {
                continue;
            }
            let is_better = best.is_none_or(|(best_missing, best_desired, _)| {
                let candidate_ratio = missing as u64 * best_desired as u64;
                let best_ratio = best_missing as u64 * desired as u64;
                candidate_ratio > best_ratio
                    || (candidate_ratio == best_ratio && missing > best_missing)
            });
            if is_better {
                best = Some((missing, desired, kind));
            }
        }
        best.map(|(_, _, kind)| kind)
    }

    fn add(&mut self, swarm: SwarmId, kind: NanobotType, slots: u32) {
        *self.desired.entry((swarm, kind)).or_default() += slots;
    }
}

/// Convert actionable work into bounded nanobot slots. Resource quantities are
/// never summed directly: one large deposit is one extraction slot, not one slot
/// per mineral.
pub fn population_demand_system(
    projection: Res<ActionableProjection>,
    territory: Res<TerritorySnapshot>,
    mut demand: ResMut<PopulationDemand>,
    access: WorkAccess,
) {
    demand.desired.clear();
    let mut haul_slots = HashMap::<(SwarmId, Entity), u32>::new();
    let live_swarms = territory.swarms().collect::<HashSet<_>>();
    for (_, opportunities) in projection.iter_regions() {
        for opportunity in opportunities {
            let owners = opportunity
                .owner
                .map(|owner| vec![owner])
                .unwrap_or_else(|| live_swarms.iter().copied().collect());
            for swarm in owners {
                if access.opportunity(swarm, opportunity.target) == WorkReachability::Unreachable {
                    continue;
                }
                let (kind, slots) = match opportunity.category {
                    OpportunityCategory::Gather
                    | OpportunityCategory::PlannedBuild
                    | OpportunityCategory::Maintenance => (NanobotType::Worker, 1),
                    OpportunityCategory::Haul => {
                        let OpportunityTarget::Haul { source, .. } = opportunity.target else {
                            continue;
                        };
                        let trips = opportunity
                            .available_work
                            .div_ceil(HAULER_CARRY_CAPACITY)
                            .max(1);
                        haul_slots
                            .entry((swarm, source))
                            .and_modify(|current| *current = (*current).max(trips))
                            .or_insert(trips);
                        continue;
                    }
                };
                demand.add(swarm, kind, slots);
            }
        }
    }
    for ((swarm, _), slots) in haul_slots {
        demand.add(swarm, NanobotType::Hauler, slots);
    }
    for swarm in live_swarms {
        demand.add(
            swarm,
            NanobotType::Defender,
            defender_population_demand(territory.tile_count(swarm), territory.threat_count(swarm)),
        );
    }
}

pub struct PopulationDemandPlugin;

impl Plugin for PopulationDemandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PopulationDemand>().add_systems(
            FixedUpdate,
            population_demand_system
                .after(RegionalAllocationSet::Project)
                .before(production_facility_pick_target_system),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demand_for(entries: &[(NanobotType, u32)]) -> PopulationDemand {
        let mut demand = PopulationDemand::default();
        for (kind, desired) in entries {
            demand.desired.insert((SwarmId::PLAYER, *kind), *desired);
        }
        demand
    }

    #[test]
    fn completely_unstaffed_small_role_beats_partially_staffed_large_role() {
        let demand = demand_for(&[
            (NanobotType::Worker, 10),
            (NanobotType::Hauler, 2),
            (NanobotType::Defender, 2),
        ]);
        let covered = HashMap::from([
            (NanobotType::Worker, 5),
            (NanobotType::Hauler, 1),
            (NanobotType::Defender, 0),
        ]);

        assert_eq!(
            demand.most_underfilled_type(SwarmId::PLAYER, &covered),
            Some(NanobotType::Defender),
        );
    }

    #[test]
    fn larger_missing_count_breaks_equal_relative_shortage() {
        let demand = demand_for(&[(NanobotType::Worker, 10), (NanobotType::Hauler, 4)]);
        let covered = HashMap::from([(NanobotType::Worker, 5), (NanobotType::Hauler, 2)]);

        assert_eq!(
            demand.most_underfilled_type(SwarmId::PLAYER, &covered),
            Some(NanobotType::Worker),
        );
    }

    #[test]
    fn stable_type_order_breaks_an_exact_shortage_tie() {
        let demand = demand_for(&[(NanobotType::Worker, 2), (NanobotType::Hauler, 2)]);

        assert_eq!(
            demand.most_underfilled_type(SwarmId::PLAYER, &HashMap::new()),
            Some(NanobotType::Worker),
        );
    }

    #[test]
    fn covered_and_zero_demand_types_do_not_produce() {
        let demand = demand_for(&[(NanobotType::Worker, 1)]);
        let covered = HashMap::from([(NanobotType::Worker, 1)]);

        assert_eq!(
            demand.most_underfilled_type(SwarmId::PLAYER, &covered),
            None,
        );
    }
}
