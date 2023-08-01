mod components;
mod consts;
mod debug;
mod groups_merge_split;
mod move_system;
mod unit_select;

pub use components::*;
pub use consts::*;
pub use debug::*;
pub use groups_merge_split::*;
pub use move_system::*;
pub use unit_select::*;

use bevy::prelude::*;

use crate::{ai::AiStateComponent, zones::ZoneComponent};

pub use self::components::{Nanobot, VelocityComponent};

#[derive(Debug, Bundle, Default)]
pub struct NanobotBundle {
    pub nanobot: Nanobot,
    pub velocity: VelocityComponent,
    pub ai_state: AiStateComponent,
}

#[derive(Debug, Default)]
pub struct NanobotPlugin {}

impl Plugin for NanobotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GroupIdCounterResource { count: 0 });

        app.add_systems(Update, separation_system)
            .add_systems(Update, velocity_system)
            .add_systems(Update, group_action_system)
            .add_systems(Update, move_velocity_system)
            .add_systems(Update, bot_debug_circle_system)
            .add_systems(Update, unit_select_system);
    }
}

#[derive(Debug, Resource)]
pub struct GroupIdCounterResource {
    pub count: u16,
}

impl GroupIdCounterResource {
    pub fn next_id(&mut self) -> u16 {
        self.count += 1;
        self.count
    }
}

#[derive(Debug, Bundle, Default)]
pub struct NanobotGroupBundle {
    pub group: NanobotGroup,
    pub spatial_bundle: SpatialBundle,
    pub zone: ZoneComponent,
}
