//! Shared exterior work geometry for movement, arrival, and continuing effects.

use bevy::prelude::*;

use super::{BUILDING_FOOTPRINT_RADIUS, DirectMovementComponent, STOP_THRESHOLD};

/// Conservative radius enclosing the rendered bodies of all three Nanobot Types.
/// Work clearance includes their silhouettes; local separation has its own tuning.
pub const INTERACTION_BODY_RADIUS: f32 = 34.0;

/// A body's center must lie outside the footprint and within four world units
/// of contact. Approaches leave two units beyond the body clearance bound,
/// allowing the movement stopping tolerance without entering the footprint.
#[derive(Debug, Clone, Copy)]
pub struct InteractionRegion {
    center: Vec2,
    rotation: Quat,
    shape: Footprint,
}

#[derive(Debug, Clone, Copy)]
enum Footprint {
    Circle(f32),
    Rectangle(Vec2),
}

impl InteractionRegion {
    /// Deposits declare their physical world-space radius independently of sprite scale.
    pub fn deposit(transform: &Transform, radius: f32) -> Self {
        Self {
            center: transform.translation.truncate(),
            rotation: Quat::IDENTITY,
            shape: Footprint::Circle(radius),
        }
    }

    pub fn structure(transform: &Transform) -> Self {
        Self {
            center: transform.translation.truncate(),
            rotation: transform.rotation,
            shape: Footprint::Rectangle(
                transform.scale.truncate().abs() * BUILDING_FOOTPRINT_RADIUS,
            ),
        }
    }

    /// True only while the entire body is outside and its surface can reach work.
    pub fn contains(self, position: Vec2) -> bool {
        let (distance, _, _) = self.surface(position);
        (INTERACTION_BODY_RADIUS - 0.001..=INTERACTION_BODY_RADIUS + 2.0 * STOP_THRESHOLD + 0.001)
            .contains(&distance)
    }

    /// Closest exterior work position; an interior start exits through its nearest edge.
    pub fn approach(self, position: Vec2) -> Vec2 {
        let (_, surface, normal) = self.surface(position);
        self.center
            + (self.rotation
                * (surface + normal * (INTERACTION_BODY_RADIUS + STOP_THRESHOLD)).extend(0.0))
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
        match self.shape {
            Footprint::Circle(radius) => {
                let normal = local.try_normalize().unwrap_or(Vec2::X);
                (local.length() - radius, normal * radius, normal)
            }
            Footprint::Rectangle(half) => {
                let closest = local.clamp(-half, half);
                let offset = local - closest;
                if let Some(normal) = offset.try_normalize() {
                    (offset.length(), closest, normal)
                } else {
                    let remaining = half - local.abs();
                    let normal = if remaining.x <= remaining.y {
                        Vec2::new(if local.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
                    } else {
                        Vec2::new(0.0, if local.y < 0.0 { -1.0 } else { 1.0 })
                    };
                    let depth = remaining.min_element();
                    (-depth, local + normal * depth, normal)
                }
            }
        }
    }
}
