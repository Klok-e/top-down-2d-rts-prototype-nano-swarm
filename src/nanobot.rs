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

use crate::zones::ZoneComponent;

pub use self::components::{Nanobot, Velocity};

#[derive(Debug, Bundle, Default)]
pub struct NanobotBundle {
    nanobot: Nanobot,
    velocity: Velocity,
}

#[derive(Debug, Default)]
pub struct NanobotPlugin {}

impl Plugin for NanobotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GroupIdCounterResource { count: 0 });

        app.add_system(separation_system)
            .add_system(velocity_system)
            .add_system(group_action_system)
            .add_system(move_velocity_system)
            .add_system(bot_debug_circle_system)
            .add_system(unit_select_system);
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
