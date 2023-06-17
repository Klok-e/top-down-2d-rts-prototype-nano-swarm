mod fps_count;
mod selected_groups_list;
mod selected_groups_system;
mod ui_events;
mod ui_interaction_system;
mod ui_setup;

pub use selected_groups_list::*;
pub use selected_groups_system::*;
pub use ui_events::*;
pub use ui_interaction_system::*;
pub use ui_setup::*;

use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::{App, Plugin},
};

use self::fps_count::fps_ui_system;

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiHandling::default())
            .add_event::<SelectedGroupsChanged>()
            .add_event::<NanobotGroupAction>()
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_startup_system(setup_ui_system)
            .add_system(mouse_scroll)
            .add_system(button_system)
            .add_system(update_selected_nanobot_groups_system)
            .add_system(check_ui_interaction)
            .add_system(fps_ui_system);
    }
}
