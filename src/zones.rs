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
}

#[derive(Debug, Default)]
pub struct ZonesPlugin {}

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_plugin(Material2dPlugin::<ZoneMaterial>::default())
            .add_system(zone_texture_update_system)
            .add_system(zone_brush_system);
    }
}
