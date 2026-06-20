pub mod button_bg_interaction;
pub mod consts;
mod fps_count;
mod ui_interaction_system;
mod ui_setup;

pub use ui_interaction_system::*;
pub use ui_setup::*;

use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::{App, Plugin, Startup, Update},
};

use self::{button_bg_interaction::button_background_system, fps_count::fps_ui_system};

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiHandling::default())
            .add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, setup_ui_system)
            .add_systems(Update, check_ui_interaction)
            .add_systems(Update, fps_ui_system)
            .add_systems(Update, button_background_system);
    }
}
