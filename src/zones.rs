mod zone_brush;

pub use zone_brush::*;

use bevy::{
    ecs::schedule::IntoScheduleConfigs,
    prelude::{Plugin, Update},
    sprite_render::Material2dPlugin,
};

use crate::intent::{brush_selection_keyboard_system, BrushSelection};

#[derive(Debug, Default)]
pub struct ZonesPlugin {}

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_plugins(Material2dPlugin::<ZoneMaterial>::default())
            .init_resource::<BrushSelection>()
            .add_systems(
                Update,
                brush_selection_keyboard_system.before(zone_brush_system),
            )
            .add_systems(Update, zone_brush_system)
            .add_systems(Update, mirror_intent_to_zone_material_system);
    }
}
