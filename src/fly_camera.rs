mod camera_move;
mod camera_zoom;

use bevy::prelude::{Plugin, Update};
pub use camera_move::*;
pub use camera_zoom::*;

#[derive(Debug)]
pub struct Camera2dFlyPlugin;

impl Plugin for Camera2dFlyPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_systems(Update, camera_2d_movement_system)
            .add_systems(Update, camera_2d_zoom_system);
    }
}
