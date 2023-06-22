mod zone_brush;

pub use zone_brush::*;

use bevy::{
    prelude::{Component, IVec2, Plugin},
    sprite::Material2dPlugin,
    utils::HashSet,
};

#[derive(Debug, Component)]
pub struct ZoneComponent {
    pub zone_points: HashSet<IVec2>,
    /// only 4 first bits are used
    pub zone_color: u32,
}

#[derive(Debug, Default)]
pub struct ZonesPlugin {}

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_plugin(Material2dPlugin::<ZoneMaterial>::default())
            .add_event::<ZoneChangedEvent>()
            .add_system(handle_zone_event_system)
            .add_system(zone_brush_system)
            .add_system(selected_zone_highlight_system);
    }
}
