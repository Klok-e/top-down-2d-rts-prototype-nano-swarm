use bevy::prelude::*;

use crate::game_settings::GameSettings;

pub const BOT_RADIUS: f32 = 50.;

#[derive(Debug, Component)]
pub struct Nanobot {}

#[derive(Debug, Component)]
pub struct MoveDestination {
    pub xy: Vec2,
}

pub fn move_velocity_system(
    mut bots: Query<(&MoveDestination, &mut Transform), With<Nanobot>>,
    game_settings: Res<GameSettings>,
) {
    let speed = game_settings.bot_speed;
    for (bot_destination, mut transform) in bots.iter_mut() {
        let dest: Vec3 = [bot_destination.xy.x, bot_destination.xy.y, 0.].into();
        let translation = transform.translation;
        let direction = dest - translation;
        transform.translation += direction.normalize() * speed.min(dest.distance(translation));
    }
}
