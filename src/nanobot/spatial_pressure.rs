//! Reusable spatial-pressure helpers shared between defender
//! spreading (issue #37) and future idle cosmetic spread.
//!
//! The defender spread model (see
//! `docs/adr/0007-defender-spatial-pressure.md`) scores every
//! visible owned Defend cell as a function of paint strength,
//! defend pressure, distance, and crowding. Crowding combines
//! two occupancy signals:
//!
//! - **physical density** -- every nanobot physically standing in
//!   the candidate cell, regardless of type or state; and
//! - **reservations** -- defenders already assigned to or holding
//!   that cell (the `(cell, Defend)` soft work slot).
//!
//! Both signals are turned into a single soft crowding multiplier
//! that never hard-rejects a cell: an extra defender lowers the
//! score but can never zero it, so very strong paint can still pull
//! a defender into an already-occupied cell when nothing better is
//! available.
//!
//! The helpers here are deliberately pure (capacity / crowding /
//! geometry math) or plain Bevy resources (`CellDensity`). They
//! carry no Defend-specific semantics so the future idle cosmetic
//! spread issue can reuse the same capacity / crowding / density /
//! de-clamping math without reaching into `defend.rs`.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::nanobot::components::Nanobot;
use crate::nanobot::gather::world_to_cell;
use crate::ZONE_BLOCK_SIZE;

/// Paint strength required to add one unit of desired occupancy
/// capacity to a Defend cell. Lower values let weak paint support
/// more defenders before crowding bites; higher values concentrate
/// defenders on strongly painted cells. Four strength per capacity
/// unit means a maximally painted cell (`PAINT_STRENGTH_CAP == 16`)
/// supports four defenders comfortably before crowding
/// significantly reduces its score, and a half-painted cell
/// supports two.
pub const DEFEND_PAINT_PER_CAPACITY_UNIT: u8 = 4;

/// Desired defender occupancy a cell supports before crowding
/// significantly reduces its score. Grows linearly with paint
/// strength and is floored at `1` so even a minimally painted
/// Defend cell can hold a single defender without self-crowding.
/// The first defender at a cell never crowding-penalises it; each
/// defender beyond the capacity lowers the score but never to zero.
pub fn paint_occupancy_capacity(strength: u8) -> u32 {
    (strength / DEFEND_PAINT_PER_CAPACITY_UNIT).max(1) as u32
}

/// Soft crowding multiplier in `(0, 1]`. The shape is
/// `capacity / (capacity + occupancy)`: never zero (so extra
/// defenders are never hard-rejected by capacity), strictly
/// decreasing in `occupancy`, and higher `capacity` (from stronger
/// paint) yields a higher factor at the same occupancy. With
/// `capacity == 1` the formula reduces to `1 / (1 + occupancy)`,
/// matching the legacy [`crate::nanobot::autonomy::SoftWorkSlots`]
/// soft-slot shape so existing gather/build/haul scoring keeps its
/// tuning while Defend scoring switches to paint-driven capacity.
pub fn crowding_factor(occupancy: u32, capacity: u32) -> f32 {
    let cap = capacity.max(1) as f32;
    cap / (cap + occupancy as f32)
}

/// Per-tick count of every nanobot physically standing in each
/// intent-grid cell, regardless of type or state. Computed by
/// [`cell_density_system`] before the defend assignment system
/// runs, and read by the defend scorer so a candidate cell's
/// crowding reflects ALL nanobots (workers, haulers, defenders)
/// physically present, not just defender reservations. The scoring
/// defender excludes its own body from its current cell's count
/// (see `DefendSelfExclusion` in `defend.rs`).
///
/// The resource is a plain `HashMap` cloneable snapshot so the
/// assignment system can read a consistent per-defender view
/// without recomputing positions mid-loop. The future idle
/// cosmetic spread issue can read the same density to de-clump
/// idle nanobots without a second pass.
#[derive(Debug, Default, Clone, Resource)]
pub struct CellDensity {
    counts: HashMap<IVec2, u32>,
}

impl CellDensity {
    /// Number of nanobots physically standing in `cell`, or `0`
    /// when no nanobot has been observed there this tick.
    pub fn density(&self, cell: IVec2) -> u32 {
        self.counts.get(&cell).copied().unwrap_or(0)
    }

    /// Number of distinct cells with at least one nanobot. Useful
    /// for tests asserting the density pass observed the world.
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// True when no cells have any nanobots.
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// Recompute [`CellDensity`] from every nanobot's world position.
/// Runs once per tick before the defend assignment system so the
/// scorer sees the post-movement physical layout. Clearing and
/// rebuilding each tick keeps the map free of stale entries for
/// cells whose nanobots have moved or died; the cost is linear in
/// the nanobot count, which is the same order the movement systems
/// already pay.
pub fn cell_density_system(
    mut density: ResMut<CellDensity>,
    bots: Query<&Transform, With<Nanobot>>,
) {
    density.counts.clear();
    for transform in &bots {
        let cell = world_to_cell(transform.translation.truncate());
        *density.counts.entry(cell).or_insert(0) += 1;
    }
}

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
    //! Pure-helper unit tests for the spatial-pressure math. The
    //! end-to-end defender spread contracts live in
    //! `tests/defend_zone_behavior.rs`; this module pins the
    //! capacity / crowding / geometry helpers the scoring is built
    //! on so a tuning change cannot silently flip a contract.

    use super::*;

    #[test]
    fn capacity_grows_with_paint_and_floors_at_one() {
        // Even the weakest active paint (strength 1) supports a
        // single defender without self-crowding.
        assert_eq!(paint_occupancy_capacity(1), 1);
        // Capacity steps up every DEFEND_PAINT_PER_CAPACITY_UNIT
        // of paint strength. Integer division means capacity
        // rises at strengths 4, 8, 12, 16 -- not at 5.
        assert_eq!(paint_occupancy_capacity(DEFEND_PAINT_PER_CAPACITY_UNIT), 1);
        assert_eq!(
            paint_occupancy_capacity(DEFEND_PAINT_PER_CAPACITY_UNIT + 1),
            1
        );
        assert_eq!(
            paint_occupancy_capacity(DEFEND_PAINT_PER_CAPACITY_UNIT * 2),
            2
        );
        // Max paint supports the full ladder of capacity units.
        assert_eq!(
            paint_occupancy_capacity(crate::intent::PAINT_STRENGTH_CAP),
            crate::intent::PAINT_STRENGTH_CAP as u32 / DEFEND_PAINT_PER_CAPACITY_UNIT as u32
        );
    }

    #[test]
    fn crowding_factor_is_one_at_zero_occupancy() {
        // The first defender at a cell never crowding-penalises it,
        // regardless of capacity.
        assert!((crowding_factor(0, 1) - 1.0).abs() < 1e-6);
        assert!((crowding_factor(0, 4) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn crowding_factor_never_hard_rejects() {
        // Crowding is a soft penalty: it stays strictly positive no
        // matter how crowded the cell is, so very strong paint can
        // still pull an extra defender in.
        let mut prev = crowding_factor(0, 2);
        for n in 1..=32u32 {
            let next = crowding_factor(n, 2);
            assert!(next > 0.0, "crowding must stay positive at n={n}");
            assert!(
                next < prev,
                "crowding must strictly decrease with occupancy at n={n}"
            );
            prev = next;
        }
    }

    #[test]
    fn crowding_factor_higher_capacity_lifts_score_at_same_occupancy() {
        // Stronger paint raises desired occupancy (capacity), so at
        // the same occupancy the stronger cell is less crowded.
        let occ = 3;
        let weak = crowding_factor(occ, 1);
        let strong = crowding_factor(occ, 4);
        assert!(
            strong > weak,
            "stronger paint must lift the crowding factor at fixed occupancy; weak={weak} strong={strong}"
        );
    }

    #[test]
    fn crowding_factor_capacity_one_matches_legacy_soft_slot_shape() {
        // With capacity == 1 the formula must reduce to the legacy
        // 1 / (1 + occupancy) shape so gather/build/haul scoring
        // keeps its existing tuning.
        for n in 0..=8u32 {
            let legacy = 1.0 / (1.0 + n as f32);
            assert!(
                (crowding_factor(n, 1) - legacy).abs() < 1e-6,
                "capacity-1 crowding must equal legacy soft-slot shape at n={n}"
            );
        }
    }

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

    #[test]
    fn cell_density_default_is_empty_and_zero() {
        let density = CellDensity::default();
        assert!(density.is_empty());
        assert_eq!(density.len(), 0);
        assert_eq!(density.density(IVec2::new(3, 4)), 0);
    }
}
