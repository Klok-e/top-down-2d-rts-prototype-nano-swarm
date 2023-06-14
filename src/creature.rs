use bevy::prelude::*;
use bevy_rapier2d::prelude::Velocity;

#[derive(Component)]
pub struct Creature {}

pub fn move_creature_system(
    mut creatures: Query<(&mut Velocity,), With<Creature>>,
    keyboard_input: Res<Input<KeyCode>>,
) {
    let x = movement_axis(&keyboard_input, KeyCode::Right, KeyCode::Left);
    let y = movement_axis(&keyboard_input, KeyCode::Up, KeyCode::Down);
    let mut vel = Vec2::new(x, y);
    if vel.length() > 0.1 {
        vel = vel.normalize() * 150.;
    }

    for (mut creature_velocity,) in creatures.iter_mut() {
        let creature_velocity: &mut Velocity = &mut *creature_velocity;
        creature_velocity.linvel = vel;
    }
}

fn movement_axis(input: &Res<Input<KeyCode>>, plus: KeyCode, minus: KeyCode) -> f32 {
    let mut axis = 0.0;
    if input.pressed(plus) {
        axis += 1.0;
    }
    if input.pressed(minus) {
        axis -= 1.0;
    }
    axis
}
