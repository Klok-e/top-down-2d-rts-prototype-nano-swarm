use bevy::{
    prelude::{Children, Color, Query, With, Without},
    sprite::Sprite,
};

use crate::{
    nanobot::{Nanobot, NanobotGroup},
    unit_select::Selected,
};

pub fn highlight_selected_system(
    mut nanobot_sprites: Query<&mut Sprite, With<Nanobot>>,
    selected_groups: Query<&Children, (With<NanobotGroup>, With<Selected>)>,
    non_selected_groups: Query<&Children, (With<NanobotGroup>, Without<Selected>)>,
) {
    let selected_col = Color::rgb(1.0, 0.0, 0.0); // Red color for selected units
    let default_col = Color::rgb(1.0, 1.0, 1.0); // White color for non-selected units

    for children in selected_groups.iter() {
        for child in children.iter() {
            let mut sprite = nanobot_sprites
                .get_mut(*child)
                .expect("Nonexistent child reference");
            sprite.color = selected_col;
        }
    }

    for children in non_selected_groups.iter() {
        for child in children.iter() {
            let mut sprite = nanobot_sprites
                .get_mut(*child)
                .expect("Nonexistent child reference");
            sprite.color = default_col;
        }
    }
}
