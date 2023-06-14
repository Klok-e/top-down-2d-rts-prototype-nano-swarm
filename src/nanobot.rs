use bevy::prelude::*;

#[derive(Debug, Component)]
pub struct Nanobot {}

#[derive(Debug, Component)]
pub struct Velocity {
    pub dxdy: Vec2,
}

pub fn move_creature_system(mut creatures: Query<(&Velocity, &mut Transform), With<Nanobot>>) {
    for (creature_velocity, mut transform) in creatures.iter_mut() {
        transform.translation +=
            Vec3::from([creature_velocity.dxdy.x, creature_velocity.dxdy.y, 0.]);
    }
}
