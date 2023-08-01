use bevy::prelude::{ResMut, Resource};

use bevy::{
    prelude::{Changed, Color, Component, Query, With},
    ui::{BackgroundColor, Interaction},
};

use super::consts::{HOVERED_BUTTON, NORMAL_BUTTON, PRESSED_BUTTON};

#[derive(Debug, Default, Resource, PartialEq, Eq)]
pub enum MouseActionMode {
    #[default]
    GroupSelectMove,
    ZoneDraw,
}

#[derive(Debug, Component)]
pub struct ZoneButton;

#[allow(clippy::type_complexity)]
pub fn zone_button_system(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<ZoneButton>),
    >,
    mut mouse_mode: ResMut<MouseActionMode>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => match mouse_mode.as_ref() {
                MouseActionMode::GroupSelectMove => {
                    *mouse_mode = MouseActionMode::ZoneDraw;
                    *bg = PRESSED_BUTTON.into();
                }
                MouseActionMode::ZoneDraw => {
                    *mouse_mode = MouseActionMode::GroupSelectMove;
                    *bg = scale_color_rgb(PRESSED_BUTTON, 0.5).into();
                }
            },
            Interaction::Hovered => {
                *bg = match mouse_mode.as_ref() {
                    MouseActionMode::GroupSelectMove => HOVERED_BUTTON.into(),
                    MouseActionMode::ZoneDraw => (HOVERED_BUTTON * 0.5).into(),
                };
            }
            Interaction::None => {
                *bg = match mouse_mode.as_ref() {
                    MouseActionMode::GroupSelectMove => NORMAL_BUTTON.into(),
                    MouseActionMode::ZoneDraw => (NORMAL_BUTTON * 0.5).into(),
                };
            }
        }
    }
}

fn scale_color_rgb(color: Color, scale: f32) -> Color {
    Color::rgba(
        color.r() * scale,
        color.g() * scale,
        color.b() * scale,
        color.a(), // preserve alpha
    )
}
