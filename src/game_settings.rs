use bevy::prelude::Resource;

#[derive(Debug, Resource)]
pub struct GameSettings {
    pub width: f32,
    pub height: f32,
}
