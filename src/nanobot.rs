mod autonomy;
mod build;
mod components;
mod consts;
mod debug;
mod gather;
mod haul;
mod move_system;
mod production;

pub use autonomy::*;
pub use build::*;
pub use components::*;
pub use consts::*;
pub use debug::*;
pub use gather::*;
pub use haul::*;
pub use move_system::*;
pub use production::*;

use bevy::prelude::*;

use crate::ai::AiStateComponent;

pub use self::components::{Nanobot, VelocityComponent};

/// Bundle for a freshly spawned nanobot. The default is a Worker
/// (the most common type for the first implementation) with zero
/// velocity and a fresh AI state. Spawners can override individual
/// fields to specialise the bot (e.g. tests spawn Haulers).
#[derive(Debug, Bundle)]
pub struct NanobotBundle {
    pub nanobot: Nanobot,
    pub nanobot_type: NanobotType,
    pub velocity: VelocityComponent,
    pub ai_state: AiStateComponent,
}

impl Default for NanobotBundle {
    fn default() -> Self {
        Self {
            nanobot: Nanobot {},
            nanobot_type: NanobotType::Worker,
            velocity: VelocityComponent::default(),
            ai_state: AiStateComponent::new(),
        }
    }
}

/// Top-level bundle for the player swarm. Holds the [`Swarm`] marker and a
/// transform used as the origin for child nanobots.
#[derive(Debug, Bundle, Default)]
pub struct SwarmBundle {
    pub swarm: Swarm,
    pub transform: Transform,
}

#[derive(Debug, Default)]
pub struct NanobotPlugin {}

impl Plugin for NanobotPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, separation_system)
            .add_systems(Update, velocity_system)
            .add_systems(Update, move_velocity_system)
            .add_systems(Update, bot_debug_circle_system);
    }
}
