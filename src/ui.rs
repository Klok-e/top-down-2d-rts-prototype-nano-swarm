pub mod button_bg_interaction;
pub mod consts;
mod fps_count;
mod selected_groups_list;
mod selected_groups_system;
mod ui_events;
mod ui_interaction_system;
mod ui_setup;
pub mod zone_button;

pub use selected_groups_list::*;
pub use selected_groups_system::*;
pub use ui_events::*;
pub use ui_interaction_system::*;
pub use ui_setup::*;

use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::{App, Plugin, Startup, Update},
};

use self::{
    button_bg_interaction::button_background_system,
    fps_count::fps_ui_system,
    zone_button::{zone_button_system, MouseActionMode},
};

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiHandling::default())
            .insert_resource(MouseActionMode::default())
            .add_event::<SelectedGroupsChanged>()
            .add_event::<NanobotGroupAction>()
            .add_plugins(FrameTimeDiagnosticsPlugin)
            .add_systems(Startup, setup_ui_system)
            .add_systems(Update, check_ui_interaction)
            .add_systems(Update, zone_button_system)
            .add_systems(Update, mouse_scroll)
            .add_systems(Update, button_system)
            .add_systems(Update, update_selected_nanobot_groups_system)
            .add_systems(Update, fps_ui_system)
            .add_systems(Update, button_background_system);
    }
}
