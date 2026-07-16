//! Workload-derived total population demand.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::nanobot::{
    ActionableProjection, HAULER_CARRY_CAPACITY, OpportunityCategory, OpportunityTarget,
    RegionalAllocationSet, SwarmId, production_facility_pick_target_system,
};

/// Desired total population by swarm, derived from discrete useful work capacity.
#[derive(Debug, Default, Resource)]
pub struct PopulationDemand {
    desired: HashMap<SwarmId, u32>,
}

impl PopulationDemand {
    pub fn desired_for(&self, swarm: SwarmId) -> u32 {
        self.desired.get(&swarm).copied().unwrap_or_default()
    }
}

/// Convert actionable work into bounded nanobot slots. Resource quantities are
/// never summed directly: one large deposit is one extraction slot, not one slot
/// per mineral.
pub fn population_demand_system(
    projection: Res<ActionableProjection>,
    mut demand: ResMut<PopulationDemand>,
) {
    demand.desired.clear();
    let mut haul_slots = HashMap::<(SwarmId, Entity), u32>::new();
    for (_, opportunities) in projection.iter_regions() {
        for opportunity in opportunities {
            let swarm = opportunity.owner.unwrap_or(SwarmId::PLAYER);
            let slots = match opportunity.category {
                OpportunityCategory::Gather
                | OpportunityCategory::PlannedBuild
                | OpportunityCategory::Maintenance => 1,
                OpportunityCategory::Defend => opportunity.available_work.max(1),
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
            *demand.desired.entry(swarm).or_default() += slots;
        }
    }
    for ((swarm, _), slots) in haul_slots {
        *demand.desired.entry(swarm).or_default() += slots;
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
