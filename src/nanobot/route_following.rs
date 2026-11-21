use bevy::prelude::*;

use crate::navigation::Navigation;

pub(super) const WAYPOINT_REACHED_DISTANCE: f32 = 0.01;

/// Tracks the waypoint selected for the current movement leg.
#[derive(Clone, Copy, Default)]
pub(super) struct RouteFollower {
    target: Option<WaypointTarget>,
}

#[derive(Clone, Copy)]
struct WaypointTarget {
    index: usize,
    approach_start: Vec2,
}

impl RouteFollower {
    pub(super) fn target_index(&self, current: usize) -> usize {
        self.target.map_or(current, |target| target.index)
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
        if let Some(WaypointTarget {
            index,
            approach_start,
        }) = self.target
        {
            let reached = waypoints.get(index).is_some_and(|point| {
                let offset = position - *point;
                let distance = offset.length();
                distance < WAYPOINT_REACHED_DISTANCE
                    || (distance <= crate::navigation::BODY_RADIUS
                        && waypoints.get(index + 1).is_some_and(|next| {
                            // A local sidestep can pass the turn without touching the waypoint.
                            // Require progress on both legs so a reversal still visits its turn.
                            let passed = offset.dot(*point - approach_start) > 0.0
                                && offset.dot(*next - *point) > 0.0;
                            (distance <= speed || passed)
                                && navigation.movement_clear(position, *next)
                        }))
            });
            if reached {
                *current = index + 1;
                self.target = None;
            } else if index < *current
                || waypoints
                    .get(index)
                    .is_none_or(|point| !navigation.movement_clear(position, *point))
            {
                self.target = None;
            }
        }
        while waypoints
            .get(*current)
            .is_some_and(|point| position.distance(*point) < WAYPOINT_REACHED_DISTANCE)
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
                    self.target = Some(WaypointTarget {
                        index: candidate,
                        approach_start: position,
                    });
                }
            }
        }
        let Some(WaypointTarget { index, .. }) = self.target else {
            return Vec2::ZERO;
        };
        let delta = waypoints[index] - position;
        delta.normalize_or_zero() * speed.max(0.0).min(delta.length())
    }
}

/// Join the closest reachable segment of a result computed from an earlier position.
pub(super) fn rejoin_route(
    position: Vec2,
    start: Vec2,
    waypoints: &mut [Vec2],
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
    let index = best.1;
    if let Some(pair) = waypoints.get_mut(index..index + 2)
        && !navigation.movement_clear(position, pair[1])
    {
        let edge = pair[1] - pair[0];
        let direction = edge.normalize_or_zero();
        let progress = (position - pair[0]).dot(direction);
        if progress > 0.0 && progress < edge.length() && progress <= crate::navigation::BODY_RADIUS
        {
            // A bot nudged to the side of a narrow aisle can return to the route at
            // its current progress instead of walking back to the same grid center.
            let join = pair[0] + direction * progress;
            if position.distance(join) <= crate::navigation::BODY_RADIUS
                && navigation.movement_clear(position, join)
                && navigation.movement_clear(join, pair[1])
            {
                pair[0] = join;
            }
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{intent::IntentGrid, navigation::Obstacle};

    #[test]
    fn replacement_route_joins_ahead_of_actual_progress() {
        let navigation = Navigation::new(&IntentGrid::new(4, 4), vec![]);
        let mut points = [
            Vec2::new(10.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(30.0, 0.0),
        ];
        let mut current = rejoin_route(Vec2::new(25.0, 1.0), Vec2::ZERO, &mut points, &navigation);
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
    fn replacement_route_rejoins_a_narrow_aisle_without_backtracking() {
        let navigation = Navigation::new(
            &IntentGrid::new(4, 4),
            vec![Obstacle::Rectangle {
                center: Vec2::new(-36.0, 36.0),
                half: Vec2::splat(36.0),
            }],
        );
        let position = Vec2::new(16.0, -31.5);
        let mut points = [
            Vec2::new(36.0, -36.0),
            Vec2::new(-36.0, -36.0),
            Vec2::new(-108.0, -36.0),
        ];
        let mut current = rejoin_route(position, position, &mut points, &navigation);
        let step = RouteFollower::default().route_step(
            position,
            &points,
            &mut current,
            5.0,
            false,
            &navigation,
        );
        assert!(
            step.y < 0.0 && step.x <= 0.0,
            "turn back into the aisle without retracing the previous leg: {step:?}"
        );
        assert!(navigation.movement_clear(position, position + step));
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
