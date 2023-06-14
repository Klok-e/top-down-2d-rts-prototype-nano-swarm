use bevy::{
    prelude::{Color, Query, With, Without},
    sprite::Sprite,
};

use crate::unit_select::Selected;

pub fn highlight_selected_system(
    mut query: Query<&mut Sprite, With<Selected>>,
    mut non_selected_query: Query<&mut Sprite, Without<Selected>>,
) {
    let selected_col = Color::rgb(1.0, 0.0, 0.0); // Red color for selected units
    let default_col = Color::rgb(1.0, 1.0, 1.0); // White color for non-selected units

    for mut sprite in query.iter_mut() {
        sprite.color = selected_col;
    }

    for mut sprite in non_selected_query.iter_mut() {
        sprite.color = default_col;
    }
}
