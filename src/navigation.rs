//! Shared physical geometry and hierarchical routes, independent of intent paint.

use bevy::prelude::*;

mod routing;
pub use routing::{
    AccessCheck, AccessStatus, Navigation, NavigationWork, Route, RouteGoal, RouteOutcome,
    RoutePriority, RouteRequestId, RouteStatus,
};

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
#[derive(Debug, Clone, Copy, PartialEq)]
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

#[cfg(test)]
mod route_tests {
    use super::*;
    use crate::{intent::IntentGrid, nanobot::SwarmId};

    #[test]
    fn shared_route_detours_around_a_wall_across_chunks() {
        let grid = IntentGrid::new(6, 6);
        let navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(36.0, 600.0),
            }],
        );
        let RouteOutcome::Found(route) = navigation.route(
            Vec2::new(-500.0, 0.0),
            Vec2::new(500.0, 0.0),
            &grid,
            SwarmId::PLAYER,
            false,
        ) else {
            panic!("wall has open ends")
        };
        assert!(route.waypoints.iter().any(|point| point.y.abs() >= 634.0));
        assert!(route.cost >= 1600.0, "detour must account for wall height");
    }
    #[test]
    fn swept_body_rejects_circle_tunnelling_and_rectangle_corner_cutting() {
        let grid = IntentGrid::new(4, 4);
        let circle = Navigation::new(&grid, vec![Obstacle::deposit(Vec2::ZERO, 10.0)]);
        assert!(!circle.segment_clear(Vec2::new(-60.0, 0.0), Vec2::new(60.0, 0.0)));
        assert!(circle.segment_clear(Vec2::new(-60.0, 44.0), Vec2::new(60.0, 44.0)));
        let rectangle = Navigation::new(&grid, vec![Obstacle::planned(Vec2::ZERO)]);
        assert!(!rectangle.segment_clear(Vec2::new(36.0, 72.0), Vec2::new(72.0, 36.0)));
        assert!(rectangle.segment_clear(Vec2::new(36.0, 90.0), Vec2::new(90.0, 36.0)));
    }

    #[test]
    fn same_fine_cell_endpoints_cannot_hide_a_deposit() {
        let grid = IntentGrid::new(2, 2);
        let navigation =
            Navigation::new(&grid, vec![Obstacle::deposit(Vec2::new(36.0, 36.0), 1.0)]);
        let start = Vec2::new(0.0, 36.0);
        let end = Vec2::new(71.0, 36.0);
        assert!(!navigation.segment_clear(start, end));
        let RouteOutcome::Found(route) =
            navigation.route(start, end, &grid, SwarmId::PLAYER, false)
        else {
            panic!("deposit has a route around it")
        };
        assert!(route.cost > 100.0);
    }

    #[test]
    fn sealed_wall_is_unreachable_but_one_cell_passage_connects_both_sides() {
        let grid = IntentGrid::new(4, 4);
        let start = Vec2::new(-180.0, 36.0);
        let end = Vec2::new(180.0, 36.0);
        let sealed = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::ZERO,
                half: Vec2::new(36.0, 1100.0),
            }],
        );
        assert!(matches!(
            sealed.route(start, end, &grid, SwarmId::PLAYER, false),
            RouteOutcome::Unreachable
        ));
        let passage = Navigation::new(
            &grid,
            vec![
                Obstacle::Rectangle {
                    center: Vec2::new(0.0, -550.0),
                    half: Vec2::new(36.0, 550.0),
                },
                Obstacle::Rectangle {
                    center: Vec2::new(0.0, 622.0),
                    half: Vec2::new(36.0, 550.0),
                },
            ],
        );
        let RouteOutcome::Found(route) = passage.route(start, end, &grid, SwarmId(7), false) else {
            panic!("72-unit gap admits a 68-unit body")
        };
        assert!((route.cost - 360.0).abs() < 0.001);
    }

    #[test]
    fn only_haulers_follow_visible_corridor_for_lower_cost() {
        use crate::intent::IntentKind;
        let mut grid = IntentGrid::new(6, 6);
        for x in -2..=1 {
            grid.paint(IVec2::new(x, 1), IntentKind::Corridor, SwarmId::PLAYER);
        }
        let navigation = Navigation::new(&grid, vec![]);
        let start = Vec2::new(-900.0, 400.0);
        let end = Vec2::new(900.0, 400.0);
        let RouteOutcome::Found(hauler) =
            navigation.route(start, end, &grid, SwarmId::PLAYER, true)
        else {
            panic!("open map")
        };
        let RouteOutcome::Found(worker) =
            navigation.route(start, end, &grid, SwarmId::PLAYER, false)
        else {
            panic!("open map")
        };
        let RouteOutcome::Found(enemy) = navigation.route(start, end, &grid, SwarmId(7), true)
        else {
            panic!("open map")
        };
        assert!(hauler.waypoints.iter().any(|p| p.y >= 512.0));
        assert!(
            hauler.cost < worker.cost * 0.8,
            "hauler={},worker={}",
            hauler.cost,
            worker.cost
        );
        assert!((worker.cost - 1800.0).abs() < 0.001);
        assert!(
            (enemy.cost - 1800.0).abs() < 0.01,
            "enemy cost={}",
            enemy.cost
        );
    }
    #[test]
    fn interaction_routes_to_a_reachable_face_when_nearest_face_is_blocked() {
        use crate::nanobot::InteractionRegion;
        let grid = IntentGrid::new(4, 4);
        let transform = Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::splat(1.125));
        let region = InteractionRegion::structure(&transform);
        let navigation = Navigation::new(
            &grid,
            vec![
                Obstacle::structure(&transform),
                Obstacle::Rectangle {
                    center: Vec2::new(-72.0, 0.0),
                    half: Vec2::new(36.0, 180.0),
                },
            ],
        );
        let start = Vec2::new(-300.0, 0.0);
        let RouteOutcome::Found(route) =
            navigation.route_to_interaction(start, region, &grid, SwarmId::PLAYER, false)
        else {
            panic!("other structure faces remain accessible")
        };
        let endpoint = *route.waypoints.last().unwrap();
        assert!(region.contains(endpoint));
        assert!(
            endpoint.x >= 0.0,
            "blocked west face must be bypassed: {endpoint:?}"
        );
    }
    #[test]
    fn disconnected_regions_in_one_chunk_can_connect_through_neighbor_chunks() {
        let grid = IntentGrid::new(4, 4);
        let navigation = Navigation::new(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::new(288.0, 288.0),
                half: Vec2::new(36.0, 288.0),
            }],
        );
        let RouteOutcome::Found(route) = navigation.route(
            Vec2::new(180.0, 180.0),
            Vec2::new(396.0, 180.0),
            &grid,
            SwarmId::PLAYER,
            false,
        ) else {
            panic!("both sides connect below the chunk")
        };
        assert!(route.waypoints.iter().any(|p| p.y <= -34.0 || p.y >= 610.0));
    }
}
