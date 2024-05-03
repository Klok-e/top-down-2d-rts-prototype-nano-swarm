use bevy::prelude::{Query, Res, Transform, With};

use crate::game_settings::GameSettings;

use super::components::Nanobot;

pub fn bot_debug_circle_system(
    game_settings: Res<GameSettings>,
    _bots: Query<&Transform, With<Nanobot>>,
    // mut shapes: ResMut<DebugShapes>,
) {
    if !game_settings.debug_draw_circles {}

    // for transform in bots.iter() {
    //     let translation = transform.translation;
    //     shapes.circle().position(translation).radius(BOT_RADIUS);
    // }
}
