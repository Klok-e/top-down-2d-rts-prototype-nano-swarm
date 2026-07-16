pub mod button_bg_interaction;
pub mod consts;
mod fps_count;
pub mod intent_layer_panel;
pub mod production_priority_panel;
mod status_panel;
mod ui_interaction_system;
mod ui_setup;

pub use ui_interaction_system::*;
pub use ui_setup::*;

use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::{App, IntoScheduleConfigs, Plugin, Startup, Update},
};

use crate::zones::zone_brush_system;

use self::{
    button_bg_interaction::button_background_system,
    fps_count::fps_ui_system,
    intent_layer_panel::{
        intent_layer_button_click_system, setup_intent_layer_panel,
        update_intent_layer_panel_highlight,
    },
    production_priority_panel::{
        ProductionPriorityDragState, production_priority_drag_system,
        setup_production_priority_panel, update_production_priority_panel,
    },
    status_panel::{setup_status_panel, update_status_panel_system},
};

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiHandling::default())
            .init_resource::<ProductionPriorityDragState>()
            .add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(
                Startup,
                (
                    setup_ui_system,
                    setup_status_panel,
                    setup_intent_layer_panel,
                    setup_production_priority_panel,
                )
                    .chain(),
            )
            // The brush reads `UiHandling::is_pointer_over_ui` as its
            // first gate; ordering the capture system before the brush
            // keeps the resource in sync with the current frame's
            // cursor state.
            .add_systems(Update, check_ui_interaction.before(zone_brush_system))
            .add_systems(Update, fps_ui_system)
            .add_systems(Update, update_status_panel_system)
            .add_systems(Update, button_background_system)
            .add_systems(Update, intent_layer_button_click_system)
            .add_systems(Update, update_intent_layer_panel_highlight)
            .add_systems(
                Update,
                (
                    production_priority_drag_system,
                    update_production_priority_panel,
                )
                    .chain(),
            );
    }
}
