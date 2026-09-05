//! Shared exterior work geometry for movement, arrival, and continuing effects.

use bevy::prelude::*;

use super::{DirectMovementComponent, STOP_THRESHOLD};
use crate::navigation::{BODY_RADIUS, Obstacle};

/// A body's center must lie outside the footprint and within four world units
/// of contact. Approaches leave two units beyond the body clearance bound,
/// allowing the movement stopping tolerance without entering the footprint.
#[derive(Debug, Clone, Copy)]
pub struct InteractionRegion {
    center: Vec2,
    rotation: Quat,
    shape: Obstacle,
}

impl InteractionRegion {
    /// Deposits declare their physical world-space radius independently of sprite scale.
    pub fn deposit(transform: &Transform, radius: f32) -> Self {
        Self {
            center: transform.translation.truncate(),
            rotation: Quat::IDENTITY,
            shape: Obstacle::deposit(Vec2::ZERO, radius),
        }
    }

    pub fn structure(transform: &Transform) -> Self {
        Self {
            center: transform.translation.truncate(),
            rotation: transform.rotation,
            shape: Obstacle::structure(&Transform {
                translation: Vec3::ZERO,
                ..*transform
            }),
        }
    }

    /// True only while the entire body is outside and its surface can reach work.
    pub fn contains(self, position: Vec2) -> bool {
        let (distance, _, _) = self.surface(position);
        (BODY_RADIUS - 0.001..=BODY_RADIUS + 2.0 * STOP_THRESHOLD + 0.001).contains(&distance)
    }

    /// Closest exterior work position; an interior start exits through its nearest edge.
    pub fn approach(self, position: Vec2) -> Vec2 {
        let (_, surface, normal) = self.surface(position);
        self.center
            + (self.rotation * (surface + normal * (BODY_RADIUS + STOP_THRESHOLD)).extend(0.0))
                .truncate()
    }

    pub fn movement_from(self, position: Vec2) -> DirectMovementComponent {
        DirectMovementComponent {
            xy: self.approach(position),
            stop_radius: STOP_THRESHOLD,
        }
    }

    fn surface(self, position: Vec2) -> (f32, Vec2, Vec2) {
        let local = (self.rotation.inverse() * (position - self.center).extend(0.0)).truncate();
        self.shape.surface(local)
    }
}
