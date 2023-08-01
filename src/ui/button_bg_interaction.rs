use bevy::{
    prelude::{Button, Changed, Component, Query, With},
    ui::{BackgroundColor, Interaction},
};

use super::consts::{HOVERED_BUTTON, NORMAL_BUTTON, PRESSED_BUTTON};

#[derive(Debug, Component)]
pub struct ButtonBgInteractiveComponent;

#[allow(clippy::type_complexity)]
pub fn button_background_system(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (
            Changed<Interaction>,
            With<Button>,
            With<ButtonBgInteractiveComponent>,
        ),
    >,
) {
    for (interaction, mut color) in &mut interaction_query {
        match *interaction {
            Interaction::Pressed => {
                *color = PRESSED_BUTTON.into();
            }
            Interaction::Hovered => {
                *color = HOVERED_BUTTON.into();
            }
            Interaction::None => {
                *color = NORMAL_BUTTON.into();
            }
        }
    }
}
