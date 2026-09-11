//! Shared gameplay rates selected for controlled AI Battle experiments.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::battle_experiment::PacingId;
use crate::nanobot::{
    CHARGE_DRAIN_PER_TICK, DEFAULT_PLANNED_WORK_TICKS, DEFENDER_ATTACK_INTERVAL_TICKS,
};

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
            construction_work_ticks: DEFAULT_PLANNED_WORK_TICKS,
            attack_interval_ticks: DEFENDER_ATTACK_INTERVAL_TICKS,
            charge_drain_per_tick: CHARGE_DRAIN_PER_TICK,
        }
    }
}

impl From<PacingId> for GameplayPacing {
    fn from(pacing: PacingId) -> Self {
        match pacing {
            PacingId::Baseline => Self::default(),
            PacingId::Deliberate => Self {
                construction_work_ticks: 90,
                attack_interval_ticks: 30,
                charge_drain_per_tick: 0.000125,
            },
        }
    }
}
