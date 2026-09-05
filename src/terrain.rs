//! Permanent terrain footprints shared by movement and construction.

use bevy::prelude::*;

use crate::navigation::Obstacle;

/// Impassable, unowned terrain. Dimensions are world units; translation locates the shape.
#[derive(Component, Debug, Clone, Copy)]
pub enum RockFormation {
    Rectangle { half: Vec2 },
    Circle { radius: f32 },
}

impl RockFormation {
    pub fn obstacle(self, transform: &Transform) -> Obstacle {
        let center = transform.translation.truncate();
        match self {
            Self::Rectangle { half } => Obstacle::Rectangle { center, half },
            Self::Circle { radius } => Obstacle::Circle { center, radius },
        }
    }
}
