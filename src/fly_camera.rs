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

/// Apply control sensitivity without moving the camera or changing its zoom.
pub fn reconcile_camera_tuning(
    world: &mut bevy::prelude::World,
    _old: &crate::tuning::Tuning,
    new: &crate::tuning::Tuning,
) {
    for mut movement in world.query::<&mut FlyCamera2d>().iter_mut(world) {
        movement.accel = new.presentation.camera_accel;
        movement.max_speed = new.presentation.camera_max_speed;
        movement.friction = new.presentation.camera_friction;
        movement.velocity = movement.velocity.clamp_length_max(movement.max_speed);
    }
    for mut zoom in world.query::<&mut CameraZoom2d>().iter_mut(world) {
        zoom.zoom_speed = new.presentation.camera_zoom_speed;
    }
}
