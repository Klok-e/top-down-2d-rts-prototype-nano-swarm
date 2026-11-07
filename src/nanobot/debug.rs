use bevy::prelude::{Query, Transform, With};

use super::components::Nanobot;

pub fn bot_debug_circle_system(
    _bots: Query<&Transform, With<Nanobot>>,
    // mut shapes: ResMut<DebugShapes>,
) {
    // for transform in bots.iter() {
    //     let translation = transform.translation;
    //     shapes.circle().position(translation).radius(BOT_RADIUS);
    // }
}
