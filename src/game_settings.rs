use std::path::Path;

use anyhow::Result;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Debug, Resource, Serialize, Deserialize)]
pub struct GameSettings {
    pub width: f32,
    pub height: f32,
    pub bot_speed: f32,
    pub debug_draw_circles: bool,
}

impl GameSettings {
    pub fn from_file_ron<P: AsRef<Path>>(path: P) -> Result<Self> {
        let str = std::fs::read_to_string(path)?;
        Ok(ron::from_str(str.as_ref())?)
    }
}
