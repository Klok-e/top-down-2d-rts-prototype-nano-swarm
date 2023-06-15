use bevy::prelude::*;

use crate::game_settings::GameSettings;
use bevy_prototype_debug_lines::DebugShapes;

pub const BOT_RADIUS: f32 = 16.;
pub const STOP_THRESHOLD: f32 = 0.01;
pub const BOT_SEPARATION_FORCE: f32 = 0.;

#[derive(Debug, Component, Default)]
pub struct NanobotGroup {}

#[derive(Debug, Bundle, Default)]
pub struct NanobotBundle {
    nanobot: Nanobot,
    velocity: Velocity,
}

#[derive(Debug, Component, Default)]
pub struct Nanobot {}

#[derive(Debug, Component)]
pub struct MoveDestination {
    pub xy: Vec2,
}

#[derive(Debug, Component, Clone, Copy, Default)]
pub struct Velocity {
    pub value: Vec2,
}

pub fn move_velocity_system(
    mut commands: Commands,
    mut bots: Query<(Entity, &MoveDestination, &Transform, &mut Velocity), With<Nanobot>>,
    game_settings: Res<GameSettings>,
) {
    let speed = game_settings.bot_speed;
    for (entity, bot_destination, transform, mut velocity) in bots.iter_mut() {
        let dest: Vec3 = [bot_destination.xy.x, bot_destination.xy.y, 0.].into();
        let translation = transform.translation;
        let direction = dest - translation;

        // Check if the distance is less than the threshold
        let distance = dest.distance(translation);
        if distance > STOP_THRESHOLD {
            let new_velocity = direction.normalize() * speed.min(distance);
            velocity.value = new_velocity.truncate();
        } else {
            commands.entity(entity).remove::<MoveDestination>();
        }
    }
}

pub fn separation_system(mut query: Query<(&Transform, &mut Velocity), With<Nanobot>>) {
    let mut combinations = query.iter_combinations_mut();

    while let Some([(transform1, mut velocity1), (transform2, mut velocity2)]) =
        combinations.fetch_next()
    {
        let distance = transform1.translation.distance(transform2.translation);
        let close_enough = BOT_RADIUS;
        if distance < close_enough {
            // Compute the vector that separates the two bots
            let separation = transform1.translation - transform2.translation;
            // Normalize the vector and scale it by the separation force
            let force = separation.normalize() * BOT_SEPARATION_FORCE;
            // Apply the separation force (this will move the bot away from its neighbor)
            velocity1.value += force.truncate();
            velocity2.value -= force.truncate();
        }
    }
}

pub fn velocity_system(mut query: Query<(&mut Velocity, &mut Transform)>) {
    for (mut velocity, mut transform) in query.iter_mut() {
        transform.translation += velocity.value.extend(0.);
        velocity.value = Vec2::ZERO;
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
