mod brush_feedback;
mod mouse_brush;
mod zone_brush;

pub use brush_feedback::{BrushFeedback, BrushHover};
pub use mouse_brush::zone_brush_system;
pub use zone_brush::*;

use bevy::{
    asset::AssetEventSystems,
    camera::CameraUpdateSystems,
    ecs::schedule::IntoScheduleConfigs,
    prelude::{Plugin, PostUpdate, Startup, Update},
    sprite_render::Material2dPlugin,
    ui::UiSystems,
};

use crate::intent::{BrushSelection, brush_selection_keyboard_system};

#[derive(Debug, Default)]
pub struct ZonesPlugin {}

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_plugins(Material2dPlugin::<ZoneMaterial>::default())
            .init_resource::<BrushSelection>()
            .init_resource::<BrushFeedback>()
            .init_resource::<crate::session::SessionRules>()
            .add_systems(Startup, brush_feedback::setup_brush_feedback)
            .add_systems(Update, brush_selection_keyboard_system)
            .add_systems(
                PostUpdate,
                (
                    zone_brush_system,
                    (
                        brush_feedback::update_brush_feedback,
                        crate::materials::update_paint_grid,
                        mirror_intent_to_zone_material_system,
                    ),
                )
                    .chain()
                    .after(CameraUpdateSystems)
                    .before(AssetEventSystems)
                    .before(UiSystems::Layout),
            );
    }
}
