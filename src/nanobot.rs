mod components;
mod consts;
mod debug;
mod move_system;

pub use components::*;
pub use consts::*;
pub use debug::*;
pub use move_system::*;

use bevy::prelude::*;

pub use self::components::{Nanobot, Velocity};

#[derive(Debug, Bundle, Default)]
pub struct NanobotBundle {
    nanobot: Nanobot,
    velocity: Velocity,
}
