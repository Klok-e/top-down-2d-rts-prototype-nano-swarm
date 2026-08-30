//! Workload-derived total population demand.

use std::collections::{HashMap, HashSet};

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
