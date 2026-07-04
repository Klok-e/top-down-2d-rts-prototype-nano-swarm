mod autonomy;
mod build;
mod charge;
mod collapse;
mod components;
mod consts;
mod debug;
mod defend;
mod demand;
mod gather;
mod haul;
mod logistics_leg;
mod maintenance;
mod move_system;
mod opponent;
mod placement;
mod planned;
mod production;
mod spatial_pressure;
mod sprites;

pub use autonomy::*;
pub use build::*;
pub use charge::*;
pub use collapse::*;
pub use components::*;
pub use consts::*;
pub use debug::*;
pub use defend::*;
pub use demand::*;
pub use gather::*;
pub use haul::*;
pub use maintenance::*;
pub use move_system::*;
pub use opponent::*;
pub use placement::*;
pub use planned::*;
pub use production::*;
pub use spatial_pressure::*;
pub use sprites::*;

use bevy::prelude::*;

use crate::ai::AiStateComponent;

pub use self::components::{Health, Nanobot, SwarmId, SwarmMember, VelocityComponent};

/// Bundle for a freshly spawned nanobot. The default is a Worker
/// (the most common type for the first implementation) with zero
/// velocity and a fresh AI state. Spawners can override individual
/// fields to specialise the bot (e.g. tests spawn Haulers).
///
/// `swarm_member` defaults to [`SwarmId::PLAYER`] so the test
/// seam helpers and any spawner that did not think about
/// ownership still pass the per-swarm intent filter (every
/// existing test uses unowned paint; legacy unowned paint is
/// visible to every swarm, so the player default works for
/// those cases). Opponent spawners and the production work
/// system overwrite `swarm_member` to the right id.
#[derive(Debug, Bundle)]
pub struct NanobotBundle {
    pub nanobot: Nanobot,
    pub nanobot_type: NanobotType,
    pub velocity: VelocityComponent,
    pub ai_state: AiStateComponent,
    pub health: Health,
    pub swarm_member: SwarmMember,
}

impl Default for NanobotBundle {
    fn default() -> Self {
        Self {
            nanobot: Nanobot {},
            nanobot_type: NanobotType::Worker,
            velocity: VelocityComponent::default(),
            ai_state: AiStateComponent::new(),
            health: Health::default(),
            swarm_member: SwarmMember::new(SwarmId::PLAYER),
        }
    }
}

/// Top-level bundle for the player swarm. Holds the [`Swarm`] marker and a
/// transform used as the origin for child nanobots.
///
/// The `SwarmId` is the player identifier; the production chain
/// reads it to stamp `SwarmMember` on freshly produced
/// nanobots, so the player swarm must always carry it.
#[derive(Debug, Bundle, Default)]
pub struct SwarmBundle {
    pub swarm: Swarm,
    pub swarm_id: SwarmId,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
    pub visibility: Visibility,
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
