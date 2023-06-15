use bevy::{
    prelude::{Children, Color, Query, With, Without},
    sprite::Sprite,
};

use crate::unit_select::Selected;

pub fn highlight_selected_system(
    mut sprites: Query<&mut Sprite>,
    selected_query: Query<&Children, With<Selected>>,
    non_selected_query: Query<&Children, Without<Selected>>,
) {
    let selected_col = Color::rgb(1.0, 0.0, 0.0); // Red color for selected units
    let default_col = Color::rgb(1.0, 1.0, 1.0); // White color for non-selected units

    for children in selected_query.iter() {
        for child in children.iter() {
            let mut sprite = sprites
                .get_mut(*child)
                .expect("Nonexistent child reference");
            sprite.color = selected_col;
        }
    }

    for children in non_selected_query.iter() {
        for child in children.iter() {
            let mut sprite = sprites
                .get_mut(*child)
                .expect("Nonexistent child reference");
            sprite.color = default_col;
        }
    }
}
