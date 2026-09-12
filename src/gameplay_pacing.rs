//! Shared gameplay rates for every shipped scenario.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Shared timing values applied to every swarm in one simulation.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplayPacing {
    pub construction_work_ticks: u32,
    pub attack_interval_ticks: u16,
    pub charge_drain_per_tick: f32,
}

impl Default for GameplayPacing {
    fn default() -> Self {
        Self {
            construction_work_ticks: 90,
            attack_interval_ticks: 30,
            charge_drain_per_tick: 0.000125,
        }
    }
}
