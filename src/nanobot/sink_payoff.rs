//! Whether a Sink Stockpile repays its Maintenance for a Production Facility.
//!
//! Terminals draw from Source and Sink Stockpiles alike, so a Sink only helps
//! when it shortens the last trip to its facility. Sinks are filled at the
//! lowest haul priority, from Haulers with nothing more urgent to do, so the
//! filling leg is treated as free and only the delivery leg counts: a Sink pays
//! when the Hauler travel it saves per second exceeds the Worker time its
//! Maintenance costs (ADR-0034).

use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*};

use crate::intent::IntentGrid;
use crate::navigation::{Navigation, RouteOutcome};
use crate::tuning::Tuning;

use super::{InteractionRegion, SwarmId};

/// Worker ticks assumed for walking to a Sink and back for each Maintenance
/// job, on top of the job itself.
const MAINTENANCE_WALK_TICKS: f32 = 3.0 * crate::SIMULATION_HZ as f32;

/// How much shorter, in walking distance, a Sink's delivery path to a facility
/// must be than the nearest own Source's for the Hauler travel it saves to
/// repay its Maintenance. A facility consuming at its full production rate is
/// assumed, and a Worker-second is valued as a Hauler-second.
pub fn sink_break_even_margin(tuning: &Tuning) -> f32 {
    let upkeep_per_tick = (tuning.maintenance.work_ticks as f32 + MAINTENANCE_WALK_TICKS)
        / tuning.maintenance.needs_threshold.max(1) as f32;
    let loads_per_tick = tuning.production.cost_per_bot as f32
        / tuning.production.ticks_per_bot.max(1) as f32
        / tuning.logistics.hauler_carry_capacity.max(1) as f32;
    // Each load from a Sink `d` nearer saves a round trip of `2 d / speed`.
    upkeep_per_tick * tuning.logistics.bot_speed / (2.0 * loads_per_tick)
}

/// Walking path lengths from a footprint to a facility, valid for one
/// navigation geometry revision. Paint does not change walking cost, so only
/// obstacle changes invalidate them.
#[derive(Debug, Default)]
pub struct DeliveryPaths {
    revision: Option<u64>,
    costs: HashMap<(u32, u32, Entity, SwarmId), Option<f32>>,
}

#[derive(SystemParam)]
pub struct SinkPayoff<'w, 's> {
    navigation: Res<'w, Navigation>,
    paths: Local<'s, DeliveryPaths>,
}

impl SinkPayoff<'_, '_> {
    /// Walking path length for a Hauler leaving the footprint `from` to reach
    /// `facility`, or `None` when no route exists.
    pub fn delivery_path(
        &mut self,
        grid: &IntentGrid,
        swarm: SwarmId,
        from: &Transform,
        facility: Entity,
        facility_transform: &Transform,
    ) -> Option<f32> {
        let revision = self.navigation.revision();
        if self.paths.revision != Some(revision) {
            self.paths.costs.clear();
            self.paths.revision = Some(revision);
        }
        let origin = from.translation.truncate();
        let key = (origin.x.to_bits(), origin.y.to_bits(), facility, swarm);
        if let Some(cost) = self.paths.costs.get(&key) {
            return *cost;
        }
        let toward = facility_transform.translation.truncate();
        let pickup = InteractionRegion::structure(from);
        let cost = std::iter::once(pickup.approach(toward))
            .chain(pickup.candidates(toward))
            .find(|start| self.navigation.point_clear(*start))
            .and_then(|start| {
                match self.navigation.route_to_interaction(
                    start,
                    InteractionRegion::structure(facility_transform),
                    grid,
                    swarm,
                    false,
                ) {
                    RouteOutcome::Found(route) => Some(route.cost),
                    RouteOutcome::Unreachable => None,
                }
            });
        self.paths.costs.insert(key, cost);
        cost
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    #[test]
    fn break_even_margin_repays_maintenance_with_saved_hauler_travel() {
        let mut tuning = Tuning::default();
        tuning.production.cost_per_bot = 5;
        tuning.production.ticks_per_bot = 120;
        tuning.logistics.hauler_carry_capacity = 10;
        tuning.logistics.bot_speed = 5.0;
        tuning.maintenance.work_ticks = 30;
        tuning.maintenance.needs_threshold = 1800;

        // Upkeep: (30 + 180) / 1800 Worker-ticks per tick. One 10-mineral load
        // every 240 ticks saves 2 d / 5 ticks, so d breaks even at 70 units.
        assert_abs_diff_eq!(sink_break_even_margin(&tuning), 70.0, epsilon = 1e-3);

        // Half the consumption means half the loads to repay the same upkeep.
        tuning.production.ticks_per_bot = 240;
        assert_abs_diff_eq!(sink_break_even_margin(&tuning), 140.0, epsilon = 1e-3);
    }
}
