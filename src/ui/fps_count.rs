use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::{Component, Query, Res, Text, With},
};

#[derive(Component)]
pub struct FpsText;

pub fn fps_ui_system(
    mut text: Query<&mut Text, With<FpsText>>,
    diagnostics: Res<DiagnosticsStore>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    if let Some(fps) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS) {
        if let Some(value) = fps.smoothed() {
            *text = Text::new(format!("FPS: {value:.2}"));
        }
    }
}
