use bevy::{
    prelude::{Commands, Entity, Query, Res, Transform, Vec2, Vec3, With},
    time::Time,
};
use rand::Rng;

use crate::{
    game_settings::GameSettings,
    nanobot::consts::{BOT_RADIUS, BOT_SEPARATION_FORCE},
};

use super::{
    components::{DirectMovementComponent, Nanobot, ProgressChecker, VelocityComponent},
    consts::STOP_THRESHOLD,
};

pub fn move_velocity_system(
    time: Res<Time>,
    mut commands: Commands,
    mut bots: Query<(
        Entity,
        &DirectMovementComponent,
        &Transform,
        &mut VelocityComponent,
        Option<&mut ProgressChecker>,
    )>,
    game_settings: Res<GameSettings>,
) {
    let speed = game_settings.bot_speed;
    for (entity, bot_destination, transform, mut velocity, progress_checker) in bots.iter_mut() {
        let dest: Vec3 = [bot_destination.xy.x, bot_destination.xy.y, 0.].into();
        let translation = transform.translation;
        let direction = dest - translation;

        // Check if the distance is less than the threshold
        let distance = dest.distance(translation);
        if distance > STOP_THRESHOLD {
            let new_velocity = direction.normalize() * speed.min(distance);
            velocity.value += new_velocity.truncate();

            // If the bot is not already moving, add a ProgressChecker
            if progress_checker.is_none() {
                commands.entity(entity).insert(ProgressChecker {
                    last_position: translation.truncate(),
                    last_update_time: time.elapsed_seconds_f64(),
                });
            }
        } else {
            commands.entity(entity).remove::<DirectMovementComponent>();
            commands.entity(entity).remove::<ProgressChecker>();
        }

        // Check if the bot has not made any significant progress for a long time
        if let Some(mut checker) = progress_checker {
            let current_time = time.elapsed_seconds_f64();
            const MAX_TIME_WITHOUT_PROGRESS: f64 = 2.0; // Maximum time without significant progress
            const MIN_PROGRESS: f32 = 1.0; // Minimum progress to reset the timer

            let progress = (checker.last_position - translation.truncate()).length();
            if progress < MIN_PROGRESS
                && current_time - checker.last_update_time > MAX_TIME_WITHOUT_PROGRESS
            {
                // The bot has not made significant progress for a long time, remove the destination
                commands.entity(entity).remove::<DirectMovementComponent>();
                commands.entity(entity).remove::<ProgressChecker>();
            } else if progress >= MIN_PROGRESS {
                // The bot has made significant progress, update the checker
                checker.last_position = translation.truncate();
                checker.last_update_time = current_time;
            }
        }
    }
}

// let path = bfs(
//     &tile_pos,
//     |IVec2 { x, y }| {
//         [
//             ivec2(*x + 1, *y),
//             ivec2(*x - 1, *y),
//             ivec2(*x, *y + 1),
//             ivec2(*x, *y - 1),
//         ]
//     },
//     |p| !curr_zone.zone_points.contains(&get_zone_from_block(*p)),
// );

pub fn separation_system(mut query: Query<(&Transform, &mut VelocityComponent), With<Nanobot>>) {
    let mut combinations = query.iter_combinations_mut();
    let mut rng = rand::thread_rng();
    const EPSILON: f32 = 1e-3;

    while let Some([(transform1, mut velocity1), (transform2, mut velocity2)]) =
        combinations.fetch_next()
    {
        let distance = transform1.translation.distance(transform2.translation);
        let close_enough = BOT_RADIUS * 2.;
        if distance < close_enough {
            // Compute the vector that separates the two bots
            let mut separation = transform1.translation - transform2.translation;

            // If separation vector is nearly zero (with the given threshold), apply a random perturbation
            if separation.length() < EPSILON {
                let angle: f32 = rng.gen_range(0.0..2.0 * std::f32::consts::PI);
                separation = Vec3::new(angle.cos(), angle.sin(), 0.0);
            }

            // Normalize the vector and scale it by the separation force
            let force = separation.normalize() * BOT_SEPARATION_FORCE;
            // Apply the separation force (this will move the bot away from its neighbor)
            velocity1.value += force.truncate();
            velocity2.value -= force.truncate();
        }
    }
}

pub fn velocity_system(mut query: Query<(&mut VelocityComponent, &mut Transform)>) {
    for (mut velocity, mut transform) in query.iter_mut() {
        transform.translation += velocity.value.extend(0.);
        velocity.value = Vec2::ZERO;
    }
}
