//! Geometry helpers for keeping nanobots inside intent-grid cells.

use bevy::prelude::*;

use crate::ZONE_BLOCK_SIZE;
use crate::nanobot::gather::world_to_cell;

/// World-space min (inclusive) and max (exclusive) corners of the
/// intent-grid cell `cell`. A cell at `(i, j)` spans
/// `[i * ZONE_BLOCK_SIZE, (i + 1) * ZONE_BLOCK_SIZE)` on x and
/// `[j * ZONE_BLOCK_SIZE, (j + 1) * ZONE_BLOCK_SIZE)` on y, so the
/// max corner is the first point that belongs to the next cell.
pub fn cell_bounds(cell: IVec2) -> (Vec2, Vec2) {
    let min = Vec2::new(
        cell.x as f32 * ZONE_BLOCK_SIZE,
        cell.y as f32 * ZONE_BLOCK_SIZE,
    );
    let max = Vec2::new(
        (cell.x + 1) as f32 * ZONE_BLOCK_SIZE,
        (cell.y + 1) as f32 * ZONE_BLOCK_SIZE,
    );
    (min, max)
}

/// Clamp `pos` to the rectangle of `cell`. A defender or idle
/// nanobot that drifts outside its assigned cell is pulled back to
/// the nearest in-cell point. Reusable by the future idle
/// cosmetic spread issue so containment math stays in one place.
pub fn clamp_point_to_cell(pos: Vec2, cell: IVec2) -> Vec2 {
    let (min, max) = cell_bounds(cell);
    Vec2::new(pos.x.clamp(min.x, max.x), pos.y.clamp(min.y, max.y))
}

/// True when `pos` lies inside the intent-grid cell `cell`
/// (min-corner inclusive, max-corner exclusive, matching
/// [`world_to_cell`]).
pub fn point_in_cell(pos: Vec2, cell: IVec2) -> bool {
    world_to_cell(pos) == cell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_bounds_span_one_zone_block() {
        let (min, max) = cell_bounds(IVec2::new(2, -1));
        assert_eq!(min, Vec2::new(2.0 * ZONE_BLOCK_SIZE, -ZONE_BLOCK_SIZE));
        assert_eq!(max, Vec2::new(3.0 * ZONE_BLOCK_SIZE, 0.0));
    }

    #[test]
    fn clamp_point_to_cell_pulls_outside_points_back_in() {
        let cell = IVec2::new(0, 0);
        // A point already inside is unchanged.
        let inside = Vec2::new(100.0, 200.0);
        assert_eq!(clamp_point_to_cell(inside, cell), inside);
        // A point past the max corner clamps to the max edge.
        let outside = Vec2::new(ZONE_BLOCK_SIZE + 50.0, -10.0);
        let clamped = clamp_point_to_cell(outside, cell);
        assert_eq!(clamped.x, ZONE_BLOCK_SIZE);
        assert_eq!(clamped.y, 0.0);
    }

    #[test]
    fn point_in_cell_matches_world_to_cell_partition() {
        // point_in_cell must agree with the same min-inclusive /
        // max-exclusive partition world_to_cell uses.
        let cell = IVec2::new(1, 1);
        let (min, max) = cell_bounds(cell);
        assert!(point_in_cell(min, cell));
        assert!(!point_in_cell(max, cell), "max corner belongs to next cell");
        assert!(point_in_cell(Vec2::new(min.x + 1.0, max.y - 1.0), cell));
        assert!(!point_in_cell(Vec2::new(min.x - 0.1, min.y), cell));
    }
}
