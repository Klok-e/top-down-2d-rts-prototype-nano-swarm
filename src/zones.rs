mod zone_brush;

pub use zone_brush::*;

use bevy::{
    prelude::{Component, IVec2, Plugin, Update},
    sprite::Material2dPlugin,
    utils::HashSet,
};

#[derive(Debug, Component, Default)]
pub struct ZoneComponent {
    pub zone_points: HashSet<IVec2>,
    /// only 4 first bits are used
    pub zone_color: u32,
}

#[derive(Debug, Default)]
pub struct ZonesPlugin {}

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_plugins(Material2dPlugin::<ZoneMaterial>::default())
            .add_event::<ZoneChangedEvent>()
            .add_systems(Update, handle_zone_event_system)
            .add_systems(Update, zone_brush_system)
            .add_systems(Update, selected_zone_highlight_system);
    }
}
