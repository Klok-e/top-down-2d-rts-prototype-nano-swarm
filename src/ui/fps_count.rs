use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::{ButtonInput, KeyCode, Query, Res, Text, Visibility, With},
};

use super::{tuning_panel::TuningEditor, ui_setup::FpsText};

pub fn fps_ui_system(
    mut counter: Query<(&mut Text, &mut Visibility), With<FpsText>>,
    diagnostics: Res<DiagnosticsStore>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Option<Res<TuningEditor>>,
) {
    let Ok((mut text, mut visibility)) = counter.single_mut() else {
        return;
    };
    if keys.just_pressed(KeyCode::F3) && !editor.is_some_and(|editor| editor.keyboard_captured()) {
        *visibility = match *visibility {
            Visibility::Hidden => Visibility::Visible,
            _ => Visibility::Hidden,
        };
    }
    if *visibility == Visibility::Hidden {
        return;
    }
    if let Some(fps) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS)
        && let Some(value) = fps.smoothed()
    {
        *text = Text::new(format!("FPS: {value:.2}"));
    }
}
