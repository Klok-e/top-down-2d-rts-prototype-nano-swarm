pub mod button_bg_interaction;
pub mod consts;
mod fps_count;
pub mod intent_layer_panel;
mod ui_interaction_system;
mod ui_setup;

pub use ui_interaction_system::*;
pub use ui_setup::*;

use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::{App, IntoScheduleConfigs, Plugin, Startup, Update},
};

use self::{
    button_bg_interaction::button_background_system,
    fps_count::fps_ui_system,
    intent_layer_panel::{
        intent_layer_button_click_system, setup_intent_layer_panel,
        update_intent_layer_panel_highlight,
    },
};

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiHandling::default())
            .add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, (setup_ui_system, setup_intent_layer_panel).chain())
            .add_systems(Update, check_ui_interaction)
            .add_systems(Update, fps_ui_system)
            .add_systems(Update, button_background_system)
            .add_systems(Update, intent_layer_button_click_system)
            .add_systems(Update, update_intent_layer_panel_highlight);
    }
}
