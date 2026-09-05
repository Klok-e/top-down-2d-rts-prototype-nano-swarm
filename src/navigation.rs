//! Shared fine-grid geometry. Navigation cells are independent of intent paint.

use bevy::prelude::*;

/// A 68-unit body fits a 72-unit passage with two units of clearance per side.
pub const CELL_WIDTH: f32 = 72.0;
pub const BODY_RADIUS: f32 = 34.0;
/// Unscaled support sprites use this world-space size; transforms carry cell sizing.
pub const STRUCTURE_SPRITE_SIZE: f32 = 64.0;

/// Snap each visible dimension to its nearest positive whole-cell size, then
/// snap the lower edges. Even-width rectangles have centers on cell boundaries.
pub fn align_structure(mut transform: Transform) -> Transform {
    let cells = (transform.scale.truncate().abs() * STRUCTURE_SPRITE_SIZE / CELL_WIDTH)
        .round()
        .max(Vec2::ONE);
    let size = cells * CELL_WIDTH;
    let min = ((transform.translation.truncate() - size / 2.0) / CELL_WIDTH).round() * CELL_WIDTH;
    transform.translation.x = min.x + size.x / 2.0;
    transform.translation.y = min.y + size.y / 2.0;
    transform.scale.x = size.x / STRUCTURE_SPRITE_SIZE;
    transform.scale.y = size.y / STRUCTURE_SPRITE_SIZE;
    transform.rotation = Quat::IDENTITY;
    transform
}

/// Physical shapes retain their world geometry rather than occupying intent cells.
#[derive(Debug, Clone, Copy)]
pub enum Obstacle {
    Rectangle { center: Vec2, half: Vec2 },
    Circle { center: Vec2, radius: f32 },
}

impl Obstacle {
    pub fn structure(transform: &Transform) -> Self {
        Self::Rectangle {
            center: transform.translation.truncate(),
            half: transform.scale.truncate().abs() * (STRUCTURE_SPRITE_SIZE / 2.0),
        }
    }

    pub fn planned(center: Vec2) -> Self {
        Self::Rectangle {
            center,
            half: Vec2::splat(CELL_WIDTH / 2.0),
        }
    }

    pub fn deposit(center: Vec2, radius: f32) -> Self {
        Self::Circle { center, radius }
    }

    /// Signed distance to the visible boundary; negative inside the obstacle.
    pub fn surface_distance(self, position: Vec2) -> f32 {
        self.surface(position).0
    }

    pub(crate) fn surface(self, position: Vec2) -> (f32, Vec2, Vec2) {
        match self {
            Self::Circle { center, radius } => {
                let offset = position - center;
                let normal = offset.try_normalize().unwrap_or(Vec2::X);
                (offset.length() - radius, center + normal * radius, normal)
            }
            Self::Rectangle { center, half } => {
                let local = position - center;
                let closest = local.clamp(-half, half);
                let offset = local - closest;
                if let Some(normal) = offset.try_normalize() {
                    (offset.length(), center + closest, normal)
                } else {
                    let remaining = half - local.abs();
                    let normal = if remaining.x <= remaining.y {
                        Vec2::new(if local.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
                    } else {
                        Vec2::new(0.0, if local.y < 0.0 { -1.0 } else { 1.0 })
                    };
                    let depth = remaining.min_element();
                    (-depth, position + normal * depth, normal)
                }
            }
        }
    }

    /// Whether a nanobot center has body clearance from the physical shape.
    pub fn admits_body(self, position: Vec2) -> bool {
        self.surface_distance(position) >= BODY_RADIUS
    }

    /// Check the candidate rectangle against actual obstacle edges, including padding.
    pub fn overlaps_rectangle(self, center: Vec2, half: Vec2, padding: f32) -> bool {
        match self {
            Self::Rectangle {
                center: other,
                half: other_half,
            } => {
                let separation = (center - other).abs() - half - other_half;
                separation.max(Vec2::ZERO).length() < padding
                    || (separation.x < 0.0 && separation.y < 0.0)
            }
            Self::Circle {
                center: other,
                radius,
            } => {
                let nearest = other.clamp(center - half, center + half);
                other.distance(nearest) < radius + padding
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_cell_passage_leaves_two_units_beyond_body_on_each_side() {
        let left = Obstacle::Rectangle {
            center: Vec2::new(-36.0, 36.0),
            half: Vec2::splat(36.0),
        };
        let right = Obstacle::Rectangle {
            center: Vec2::new(108.0, 36.0),
            half: Vec2::splat(36.0),
        };
        let center = Vec2::new(36.0, 36.0);
        assert!(left.admits_body(center) && right.admits_body(center));
        assert!((left.surface_distance(center) - 36.0).abs() < 0.001);
        assert!((right.surface_distance(center) - 36.0).abs() < 0.001);
        assert!(!left.admits_body(Vec2::new(33.0, 36.0)));
        assert!(!right.admits_body(Vec2::new(39.0, 36.0)));
    }

    #[test]
    fn deposit_clearance_uses_circle_instead_of_intent_cell_or_square() {
        let deposit = Obstacle::deposit(Vec2::new(100.0, 100.0), 50.0);
        assert!(deposit.admits_body(Vec2::new(160.0, 160.0)));
        assert!(!deposit.admits_body(Vec2::new(183.0, 100.0)));
        assert!(deposit.admits_body(Vec2::new(184.0, 100.0)));
    }

    #[test]
    fn rectangle_corners_cannot_overlap_despite_separated_centers() {
        let structure = Obstacle::Rectangle {
            center: Vec2::ZERO,
            half: Vec2::splat(36.0),
        };
        assert!(structure.overlaps_rectangle(Vec2::new(60.0, 60.0), Vec2::splat(36.0), 0.0));
        assert!(!structure.overlaps_rectangle(Vec2::new(72.0, 72.0), Vec2::splat(36.0), 0.0));
    }

    #[test]
    fn authored_negative_nonuniform_dimensions_snap_to_whole_cell_edges() {
        let aligned = align_structure(
            Transform::from_xyz(-155.0, -110.0, 5.0).with_scale(Vec3::new(-2.0, 3.0, 1.0)),
        );
        assert!((aligned.translation - Vec3::new(-144.0, -108.0, 5.0)).length() < 0.001);
        assert!((aligned.scale.truncate() * 64.0 - Vec2::new(144.0, 216.0)).length() < 0.001);
    }
}
