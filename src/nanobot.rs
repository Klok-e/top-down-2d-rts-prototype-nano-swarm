mod components;
mod consts;
mod debug;
mod move_system;

pub use components::*;
pub use consts::*;
pub use debug::*;
pub use move_system::*;

use bevy::prelude::*;

use crate::ai::AiStateComponent;

pub use self::components::{Nanobot, VelocityComponent};

#[derive(Debug, Bundle, Default)]
pub struct NanobotBundle {
    pub nanobot: Nanobot,
    pub velocity: VelocityComponent,
    pub ai_state: AiStateComponent,
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
