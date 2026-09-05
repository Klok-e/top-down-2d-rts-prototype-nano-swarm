//! Shared exterior work geometry for movement, arrival, and continuing effects.

use bevy::prelude::*;

use super::{DirectMovementComponent, STOP_THRESHOLD};
use crate::navigation::{BODY_RADIUS, CELL_WIDTH, Obstacle};

/// A body's center must lie outside the footprint and within four world units
/// of contact. Approaches leave two units beyond the body clearance bound,
/// allowing the movement stopping tolerance without entering the footprint.
#[derive(Debug, Clone, Copy, PartialEq)]
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
            speed: None,
            xy: self.approach(position),
            stop_radius: STOP_THRESHOLD,
            interaction: Some(self),
        }
    }

    /// Exterior approaches projected from fine cells around the whole footprint.
    /// Include the closest point without requiring a work position to coincide
    /// with a cell center in the thin interaction band.
    pub(crate) fn candidates(self, start: Vec2) -> Vec<Vec2> {
        let half = match self.shape {
            Obstacle::Circle { radius, .. } => Vec2::splat(radius),
            Obstacle::Rectangle { half, .. } => {
                let x = (self.rotation * Vec3::X).truncate().abs();
                let y = (self.rotation * Vec3::Y).truncate().abs();
                x * half.x + y * half.y
            }
        } + Vec2::splat(BODY_RADIUS + CELL_WIDTH);
        let min = ((self.center - half) / CELL_WIDTH).floor().as_ivec2();
        let max = ((self.center + half) / CELL_WIDTH).ceil().as_ivec2();
        let mut candidates = vec![self.approach(start)];
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let point = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
                let distance = self.surface(point).0;
                if (0.0..=BODY_RADIUS + CELL_WIDTH * 1.5).contains(&distance) {
                    let candidate = self.approach(point);
                    if candidates
                        .iter()
                        .all(|old| old.distance_squared(candidate) > 0.01)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        candidates
    }

    fn surface(self, position: Vec2) -> (f32, Vec2, Vec2) {
        let local = (self.rotation.inverse() * (position - self.center).extend(0.0)).truncate();
        self.shape.surface(local)
    }
}
