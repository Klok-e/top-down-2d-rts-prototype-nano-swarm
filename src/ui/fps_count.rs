use bevy::{
    diagnostic::{Diagnostics, FrameTimeDiagnosticsPlugin},
    prelude::{Component, Query, Res, With},
    text::Text,
};

#[derive(Component)]
pub struct FpsText;

pub fn fps_ui_system(mut text: Query<&mut Text, With<FpsText>>, diagnostics: Res<Diagnostics>) {
    let mut text = text.single_mut();
    if let Some(fps) = diagnostics.get(FrameTimeDiagnosticsPlugin::FPS) {
        if let Some(value) = fps.smoothed() {
            // Update the value of the second section
            text.sections[1].value = format!("{value:.2}");
        }
    }
}
