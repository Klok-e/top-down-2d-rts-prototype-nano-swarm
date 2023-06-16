use bevy::prelude::{Query, Res, ResMut, Transform, With};
use bevy_prototype_debug_lines::DebugShapes;

use crate::game_settings::GameSettings;

use super::{components::Nanobot, consts::BOT_RADIUS};

pub fn bot_debug_circle_system(
    game_settings: Res<GameSettings>,
    bots: Query<&Transform, With<Nanobot>>,
    mut shapes: ResMut<DebugShapes>,
) {
    if !game_settings.debug_draw_circles {
        return;
    }

    for transform in bots.iter() {
        let translation = transform.translation;
        shapes.circle().position(translation).radius(BOT_RADIUS);
    }
}
