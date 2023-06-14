use bevy::prelude::*;

use crate::game_settings::GameSettings;
use bevy_prototype_debug_lines::DebugShapes;

pub const BOT_RADIUS: f32 = 16.;
pub const STOP_THRESHOLD: f32 = 0.01;

#[derive(Debug, Component)]
pub struct Nanobot {}

#[derive(Debug, Component)]
pub struct MoveDestination {
    pub xy: Vec2,
}

pub fn move_velocity_system(
    mut commands: Commands,
    mut bots: Query<(Entity, &MoveDestination, &mut Transform), With<Nanobot>>,
    game_settings: Res<GameSettings>,
) {
    let speed = game_settings.bot_speed;
    for (entity, bot_destination, mut transform) in bots.iter_mut() {
        let dest: Vec3 = [bot_destination.xy.x, bot_destination.xy.y, 0.].into();
        let translation = transform.translation;
        let direction = dest - translation;

        // Check if the distance is less than the threshold
        let distance = dest.distance(translation);
        if distance > STOP_THRESHOLD {
            transform.translation += direction.normalize() * speed.min(distance);
        } else {
            commands.entity(entity).remove::<MoveDestination>();
        }
    }
}

pub fn bot_debug_circle_system(
    bots: Query<&Transform, With<Nanobot>>,
    mut shapes: ResMut<DebugShapes>,
) {
    for transform in bots.iter() {
        let translation = transform.translation;
        shapes.circle().position(translation).radius(BOT_RADIUS);
    }
}
