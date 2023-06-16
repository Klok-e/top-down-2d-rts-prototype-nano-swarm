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

use bevy::prelude::{App, Plugin};

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiHandling::default());

        app.add_event::<SelectedGroupsChanged>()
            .add_event::<NanobotGroupAction>()
            .add_startup_system(setup_ui_system)
            .add_system(mouse_scroll)
            .add_system(button_system)
            .add_system(update_selected_nanobot_groups_system)
            .add_system(check_ui_interaction);
    }
}
