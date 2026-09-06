use bevy::prelude::*;

use crate::navigation::Navigation;

/// A selected shortcut remains a target until actual movement reaches it.
#[derive(Clone, Copy, Default)]
pub(super) struct RouteFollower {
    target: Option<usize>,
}

impl RouteFollower {
    pub(super) fn target_index(&self, current: usize) -> usize {
        self.target.unwrap_or(current)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn route_step(
        &mut self,
        position: Vec2,
        waypoints: &[Vec2],
        current: &mut usize,
        speed: f32,
        hauler: bool,
        navigation: &Navigation,
    ) -> Vec2 {
        if let Some(target) = self.target {
            if waypoints
                .get(target)
                .is_some_and(|point| position.distance(*point) < 0.01)
            {
                *current = target + 1;
                self.target = None;
            } else if target < *current
                || waypoints
                    .get(target)
                    .is_none_or(|point| !navigation.movement_clear(position, *point))
            {
                self.target = None;
            }
        }
        while waypoints
            .get(*current)
            .is_some_and(|point| position.distance(*point) < 0.01)
        {
            *current += 1;
        }
        if *current >= waypoints.len() {
            return Vec2::ZERO;
        }
        if self.target.is_none() {
            let end = (*current + 8).min(waypoints.len());
            for candidate in *current..end {
                if hauler && candidate > *current {
                    let origin = if *current == 0 {
                        position
                    } else {
                        waypoints[*current - 1]
                    };
                    let direction = (waypoints[*current] - origin).normalize_or_zero();
                    let segment = waypoints[candidate] - waypoints[candidate - 1];
                    if direction != Vec2::ZERO
                        && (direction.perp_dot(segment).abs() > 0.001
                            || direction.dot(segment) < 0.0)
                    {
                        break;
                    }
                    // A duplicate initial point must not hide a later turn.
                    if direction == Vec2::ZERO && segment.length_squared() > 0.000_001 {
                        break;
                    }
                }
                if navigation.movement_clear(position, waypoints[candidate]) {
                    self.target = Some(candidate);
                }
            }
        }
        let Some(target) = self.target else {
            return Vec2::ZERO;
        };
        let delta = waypoints[target] - position;
        delta.normalize_or_zero() * speed.max(0.0).min(delta.length())
    }
}

/// Join the closest reachable segment of a result computed from an earlier position.
pub(super) fn rejoin_index(
    position: Vec2,
    start: Vec2,
    waypoints: &[Vec2],
    navigation: &Navigation,
) -> usize {
    let mut previous = start;
    let mut best = (f32::INFINITY, 0);
    for (index, &end) in waypoints.iter().enumerate() {
        let segment = end - previous;
        let fraction = if segment.length_squared() > 0.0001 {
            ((position - previous).dot(segment) / segment.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = position.distance_squared(previous + segment * fraction);
        if distance < best.0 && navigation.movement_clear(position, end) {
            best = (distance, index);
        }
        previous = end;
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{intent::IntentGrid, navigation::Obstacle};

    #[test]
    fn replacement_route_joins_ahead_of_actual_progress() {
        let navigation = Navigation::new(&IntentGrid::new(4, 4), vec![]);
        let points = [
            Vec2::new(10.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(30.0, 0.0),
        ];
        let mut current = rejoin_index(Vec2::new(25.0, 1.0), Vec2::ZERO, &points, &navigation);
        let step = RouteFollower::default().route_step(
            Vec2::new(25.0, 1.0),
            &points,
            &mut current,
            4.0,
            false,
            &navigation,
        );
        assert!(
            step.x > 0.0,
            "published replacement must not send a traveller back to its old start"
        );
    }

    #[test]
    fn collinear_nodes_do_not_reduce_speed_and_final_arrival_does_not_overshoot() {
        let navigation = Navigation::new(&IntentGrid::new(4, 4), vec![]);
        let points = [
            Vec2::new(1.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(10.0, 0.0),
        ];
        for hauler in [false, true] {
            let mut follower = RouteFollower::default();
            let mut current = 0;
            let mut position = Vec2::ZERO;
            for expected_x in [4.0, 8.0, 10.0] {
                position +=
                    follower.route_step(position, &points, &mut current, 4.0, hauler, &navigation);
                assert!((position.x - expected_x).abs() < 0.001);
                assert!(position.y.abs() < 0.001);
            }
            let step =
                follower.route_step(position, &points, &mut current, 4.0, hauler, &navigation);
            assert!(step.length() < 0.001);
            assert_eq!(current, points.len());
        }
    }

    #[test]
    fn rejected_movement_does_not_consume_route_or_return_to_skipped_nodes() {
        let navigation = Navigation::new(&IntentGrid::new(4, 4), vec![]);
        let points = [Vec2::new(1.0, 0.0), Vec2::new(10.0, 0.0)];
        let mut follower = RouteFollower::default();
        let mut current = 0;
        for position in [Vec2::ZERO, Vec2::ZERO, Vec2::new(4.0, 0.0)] {
            let step =
                follower.route_step(position, &points, &mut current, 4.0, false, &navigation);
            assert!((step.x - 4.0).abs() < 0.001);
            assert_eq!(current, 0);
        }
    }

    #[test]
    fn hauler_preserves_a_corridor_turn_in_open_space() {
        let navigation = Navigation::new(&IntentGrid::new(4, 4), vec![]);
        let points = [Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        let mut hauler = RouteFollower::default();
        let mut ordinary = RouteFollower::default();
        let haul_step = hauler.route_step(Vec2::ZERO, &points, &mut 0, 4.0, true, &navigation);
        let ordinary_step =
            ordinary.route_step(Vec2::ZERO, &points, &mut 0, 4.0, false, &navigation);
        assert!((haul_step.x - 4.0).abs() < 0.001);
        assert!(haul_step.y.abs() < 0.001);
        assert!((ordinary_step.x - 2.828_427).abs() < 0.001);
        assert!((ordinary_step.y - 2.828_427).abs() < 0.001);
    }

    #[test]
    fn duplicate_waypoints_do_not_stall_or_hide_a_hauler_turn() {
        let navigation = Navigation::new(&IntentGrid::new(4, 4), vec![]);
        let points = [
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
        ];
        let mut current = 0;
        let step = RouteFollower::default().route_step(
            Vec2::ZERO,
            &points,
            &mut current,
            4.0,
            true,
            &navigation,
        );
        assert!((step.x - 4.0).abs() < 0.001);
        assert!(step.y.abs() < 0.001);
    }

    #[test]
    fn newly_blocked_shortcut_is_rechecked_before_moving() {
        let grid = IntentGrid::new(4, 4);
        let mut navigation = Navigation::new(&grid, vec![]);
        let points = [Vec2::new(0.0, 200.0), Vec2::new(200.0, 200.0)];
        let mut follower = RouteFollower::default();
        let mut current = 0;
        let first = follower.route_step(Vec2::ZERO, &points, &mut current, 4.0, false, &navigation);
        assert!((first.x - 2.828_427).abs() < 0.001);
        navigation.refresh(
            &grid,
            vec![Obstacle::Rectangle {
                center: Vec2::new(100.0, 100.0),
                half: Vec2::splat(20.0),
            }],
        );
        let step = follower.route_step(Vec2::ZERO, &points, &mut current, 4.0, false, &navigation);
        assert!(step.x.abs() < 0.001);
        assert!((step.y - 4.0).abs() < 0.001);
    }
}
