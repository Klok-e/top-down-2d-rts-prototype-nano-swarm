//! Hauler route-cost planning over the intent grid.
//!
//! Logistics Corridor paint is a soft cost field for haulers. The
//! planner keeps normal cells traversable, discounts visible owned
//! corridor cells, and returns ordinary waypoints for the movement
//! systems to follow.

use bevy::prelude::{IVec2, Vec2};
use pathfinding::prelude::astar;

use crate::{
    ai::get_world_from_zone,
    intent::{IntentGrid, IntentKind},
    nanobot::{gather::world_to_cell, SwarmId},
    ZONE_BLOCK_SIZE,
};

const COST_SCALE: u32 = 1_000;
const CARDINAL_STEP_COST: u32 = COST_SCALE;
const DIAGONAL_STEP_COST: u32 = 1_414;
const CORRIDOR_MIN_MULTIPLIER_SCALED: u32 = 350;
const NORMAL_MULTIPLIER_SCALED: u32 = COST_SCALE;

/// Cost multiplier for owned Logistics Corridor paint.
pub const CORRIDOR_MIN_COST_MULTIPLIER: f32 =
    CORRIDOR_MIN_MULTIPLIER_SCALED as f32 / COST_SCALE as f32;

/// A planned route between two world positions.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedRoute {
    pub waypoints: Vec<Vec2>,
    pub cost: f32,
}

/// Route cost between two world positions. Falls back to direct
/// Euclidean distance when either endpoint cannot be represented on
/// the finite intent grid.
pub fn hauler_route_cost(start: Vec2, end: Vec2, grid: &IntentGrid, swarm: SwarmId) -> f32 {
    plan_hauler_route(start, end, grid, swarm)
        .map(|route| route.cost)
        .unwrap_or_else(|| start.distance(end))
}

/// Plan a hauler route over 8-neighbour intent cells.
pub fn plan_hauler_route(
    start: Vec2,
    end: Vec2,
    grid: &IntentGrid,
    swarm: SwarmId,
) -> Option<PlannedRoute> {
    let start_cell = world_to_cell(start);
    let end_cell = world_to_cell(end);
    if !grid.in_bounds(start_cell) || !grid.in_bounds(end_cell) {
        return None;
    }

    if start_cell == end_cell {
        return Some(PlannedRoute {
            waypoints: vec![end],
            cost: start.distance(end),
        });
    }

    let (cells, scaled_cost) = astar(
        &start_cell,
        |cell| route_successors(*cell, grid, swarm),
        |cell| octile_heuristic_scaled(*cell, end_cell),
        |cell| *cell == end_cell,
    )?;

    let mut waypoints: Vec<Vec2> = cells
        .iter()
        .copied()
        .skip(1)
        .filter(|cell| *cell != end_cell)
        .map(get_world_from_zone)
        .collect();
    waypoints.push(end);

    Some(PlannedRoute {
        waypoints,
        cost: scaled_cost as f32 / COST_SCALE as f32 * ZONE_BLOCK_SIZE,
    })
}

fn route_successors(cell: IVec2, grid: &IntentGrid, swarm: SwarmId) -> Vec<(IVec2, u32)> {
    let mut out = Vec::with_capacity(8);
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let next = cell + IVec2::new(dx, dy);
            if !grid.in_bounds(next) {
                continue;
            }
            let step = if dx != 0 && dy != 0 {
                DIAGONAL_STEP_COST
            } else {
                CARDINAL_STEP_COST
            };
            let multiplier = traversal_multiplier_scaled(next, grid, swarm);
            out.push((next, (step * multiplier).div_ceil(COST_SCALE)));
        }
    }
    out
}

fn traversal_multiplier_scaled(cell: IVec2, grid: &IntentGrid, swarm: SwarmId) -> u32 {
    let Some(intent_cell) = grid.cell(cell) else {
        return NORMAL_MULTIPLIER_SCALED;
    };
    if !intent_cell.visible_to(IntentKind::Corridor, swarm) {
        return NORMAL_MULTIPLIER_SCALED;
    }
    CORRIDOR_MIN_MULTIPLIER_SCALED
}

fn octile_heuristic_scaled(from: IVec2, to: IVec2) -> u32 {
    let delta = (to - from).abs();
    let diagonal = delta.x.min(delta.y) as u32;
    let straight = delta.x.max(delta.y) as u32 - diagonal;
    let base = diagonal * DIAGONAL_STEP_COST + straight * CARDINAL_STEP_COST;
    (base * CORRIDOR_MIN_MULTIPLIER_SCALED).div_ceil(COST_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::IntentKind;

    #[test]
    fn unpainted_route_uses_diagonal_step_cost() {
        let grid = IntentGrid::new(8, 8);
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(2.0 * ZONE_BLOCK_SIZE, 2.0 * ZONE_BLOCK_SIZE);

        let route = plan_hauler_route(start, end, &grid, SwarmId::PLAYER).unwrap();

        assert!((route.cost - 2.0 * 1.414 * ZONE_BLOCK_SIZE).abs() < ZONE_BLOCK_SIZE * 0.01);
    }

    #[test]
    fn painted_corridor_has_fixed_discount() {
        let start = Vec2::ZERO;
        let end = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
        let mut grid = IntentGrid::new(8, 8);
        grid.paint(IVec2::new(1, 0), IntentKind::Corridor);

        let painted = hauler_route_cost(start, end, &grid, SwarmId::PLAYER);
        let normal = hauler_route_cost(start, end, &IntentGrid::new(8, 8), SwarmId::PLAYER);
        assert!(painted < normal);
    }

    #[test]
    fn enemy_owned_corridor_does_not_discount_route() {
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
        let painted = IVec2::new(1, 0);
        let unpainted = IntentGrid::new(8, 8);
        let mut enemy = IntentGrid::new(8, 8);
        enemy.paint_owned(painted, IntentKind::Corridor, Some(SwarmId(99)));

        let normal_cost = hauler_route_cost(start, end, &unpainted, SwarmId::PLAYER);
        let enemy_cost = hauler_route_cost(start, end, &enemy, SwarmId::PLAYER);

        assert_eq!(enemy_cost, normal_cost);
    }

    #[test]
    fn unowned_legacy_corridor_discounts_for_any_swarm() {
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0);
        let painted = IVec2::new(1, 0);
        let unpainted = IntentGrid::new(8, 8);
        let mut shared = IntentGrid::new(8, 8);
        shared.paint(painted, IntentKind::Corridor);

        let normal_cost = hauler_route_cost(start, end, &unpainted, SwarmId(42));
        let shared_cost = hauler_route_cost(start, end, &shared, SwarmId(42));

        assert!(shared_cost < normal_cost);
    }

    #[test]
    fn useful_corridor_detour_can_beat_shorter_unpainted_route() {
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(4.0 * ZONE_BLOCK_SIZE, 0.0);
        let mut grid = IntentGrid::new(12, 12);
        for cell in [
            IVec2::new(0, 1),
            IVec2::new(1, 1),
            IVec2::new(2, 1),
            IVec2::new(3, 1),
            IVec2::new(4, 1),
        ] {
            grid.paint(cell, IntentKind::Corridor);
        }

        let route = plan_hauler_route(start, end, &grid, SwarmId::PLAYER).unwrap();

        assert!(route.waypoints.iter().any(|p| world_to_cell(*p).y == 1));
    }
}
