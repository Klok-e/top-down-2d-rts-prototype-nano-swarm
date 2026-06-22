//! Source Stockpile placement algorithm (issue #24).
//!
//! Pure helpers (ring, jitter, containment, overlap, score) plus
//! [`find_source_stockpile_placement`] which runs the full
//! pipeline. The helpers have no `Res`/`Query` access, so the
//! algorithm is trivially unit-testable and the demand-system
//! behaviour tests can focus on the integration with the rest
//! of the simulation.
//!
//! ## Why pure
//!
//! The "stable across ticks" contract comes for free from
//! being a pure function of the deposit, the grid, and the
//! obstacle set: the same inputs always produce the same
//! answer, so a follow-up planning pass for the same deposit
//! lands at the same position. The "deterministic jitter"
//! contract is pinned by a `splitmix64`-based hash of
//! `(angle_index, deposit_cell)`; the seed is a function of
//! the deposit's cell, not the tick counter, so jitter is
//! stable across ticks too.

use bevy::prelude::*;

use crate::nanobot::gather::world_to_cell;

/// SplitMix64 mix constant. Used both as the initial increment
/// and as the multiplier in [`mix_hash`].
const SPLITMIX64_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Distance from the Resource Deposit's center where Source
/// Stockpile candidates are generated. Picked to keep the
/// structure inside the deposit's intent grid cell (96 + max
/// jitter 16 = 112, well within 256 half-cell width) and to
/// leave a non-trivial gap between the deposit's edge and
/// the planned structure's edge.
pub const SOURCE_STOCKPILE_PLACEMENT_RADIUS: f32 = 96.0;

/// Number of candidates generated on the placement ring.
/// Eight gives a candidate every 45 degrees, which is dense
/// enough that the haul-direction bias has a real choice
/// while keeping the per-deposit filter pass O(count).
pub const SOURCE_STOCKPILE_PLACEMENT_COUNT: usize = 8;

/// Maximum absolute offset (world units) added to a
/// candidate's ring position by the deterministic jitter.
/// Picked so that all jittered candidates still land inside
/// the deposit's intent grid cell (ring radius 96 + jitter
/// 16 = 112 < 256 half-cell) and so that successive
/// planning passes do not collapse to the same point.
pub const SOURCE_STOCKPILE_JITTER_AMPLITUDE: f32 = 16.0;

/// Extra space required between Source Stockpile footprints
/// beyond the sum of half-footprints. Models the "keep a
/// little space from deposits and other structures" user
/// story from the PRD: the swarm's support structures
/// should not pile on top of each other.
pub const SOURCE_STOCKPILE_PADDING: f32 = 16.0;

/// Half-footprint of a Source Stockpile. The
/// [`crate::nanobot::planned::PLANNED_STRUCTURE_FOOTPRINT`]
/// is the full footprint; this is the radius used by the
/// overlap test.
pub const SOURCE_STOCKPILE_FOOTPRINT_RADIUS: f32 =
    crate::nanobot::planned::PLANNED_STRUCTURE_FOOTPRINT / 2.0;

/// Shared half-footprint for planned and completed support
/// structures. Issue #34 makes placement use one footprint
/// rule for Planned Structures, Stockpiles, Production
/// Facilities, and Chargers.
pub const BUILDING_FOOTPRINT_RADIUS: f32 =
    crate::nanobot::planned::PLANNED_STRUCTURE_FOOTPRINT / 2.0;

/// Shared padding gap between support structure footprints and
/// between support structures and Resource Deposits.
pub const BUILDING_FOOTPRINT_PADDING: f32 = 16.0;

/// Maximum in-cell offset used when searching for a valid
/// Build-Zone placement. `ZONE_BLOCK_SIZE / 2 - radius - padding`
/// keeps candidates inside the painted cell with room for gap.
pub const BUILD_ZONE_PLACEMENT_MAX_OFFSET: f32 =
    crate::ZONE_BLOCK_SIZE / 2.0 - BUILDING_FOOTPRINT_RADIUS - BUILDING_FOOTPRINT_PADDING;

/// Generate `count` evenly spaced angles in radians around
/// the full circle. The first angle is 0 (east). The list
/// is empty when `count == 0`. Exposed for tests that want
/// to enumerate the same ring the algorithm uses.
pub fn placement_angles(count: usize) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }
    let step = std::f32::consts::TAU / count as f32;
    (0..count).map(|i| i as f32 * step).collect()
}

/// World position of a candidate on the placement ring at
/// `angle` (radians) around `deposit_pos`. The first
/// candidate (`angle = 0`) is east of the deposit.
pub fn ring_position(deposit_pos: Vec2, radius: f32, angle: f32) -> Vec2 {
    deposit_pos + Vec2::new(angle.cos() * radius, angle.sin() * radius)
}

/// Deterministic jitter offset for a candidate at
/// `angle_index` (0..count) around a deposit whose world
/// position falls in `deposit_cell`. The same
/// `(angle_index, deposit_cell)` pair always produces the
/// same offset, so repeated planning passes for the same
/// deposit are stable across ticks. The hash is a pure
/// function of the inputs; it does not consult any tick
/// counter, global state, or random source.
///
/// `amplitude` is the maximum absolute value of the offset
/// on either axis. The returned vector has both components
/// in `[-amplitude, +amplitude]`. A non-positive `amplitude`
/// collapses to a zero offset (still deterministic).
pub fn deterministic_jitter(angle_index: u32, deposit_cell: IVec2, amplitude: f32) -> Vec2 {
    if amplitude <= 0.0 {
        return Vec2::ZERO;
    }
    let seed = mix_hash(angle_index, deposit_cell);
    let h1 = splitmix64(seed);
    let h2 = splitmix64(seed.wrapping_add(SPLITMIX64_MIX));
    // Map the high 24 bits of the hash to `[-1, +1]` -- the
    // f32 mantissa width, which is enough for the small world
    // distances we work with and gives stable rounding.
    let dx = map_to_unit(h1) * 2.0 - 1.0;
    let dy = map_to_unit(h2) * 2.0 - 1.0;
    Vec2::new(dx * amplitude, dy * amplitude)
}

/// Mix `angle_index` and `deposit_cell` into a 64-bit seed
/// for the hash. The cell bits are stored in the high
/// 32 bits; the angle index is mixed through a multiplier
/// so adjacent angle indices still spread the seeds.
fn mix_hash(angle_index: u32, deposit_cell: IVec2) -> u64 {
    let cell_bits = ((deposit_cell.x as i64 as u64) << 32) | (deposit_cell.y as i64 as u64);
    cell_bits ^ (angle_index as u64).wrapping_mul(SPLITMIX64_MIX)
}

/// SplitMix64 hash step. Fast, well-distributed, and pure
/// (no global state, no platform-dependent behavior). Used
/// in Bevy's own deterministic scheduling code, so the
/// project is already familiar with the family.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(SPLITMIX64_MIX);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Map a `u64` hash to `[0, 1)` using the top 24 bits (the
/// precision range of an `f32` mantissa). The high bits are
/// the most-mixed output of `splitmix64`, so the result has
/// a uniform distribution in the unit interval.
fn map_to_unit(x: u64) -> f32 {
    let top = (x >> 40) as u32;
    (top as f32) / ((1u32 << 24) as f32)
}

/// True when `pos` is inside one of the `gather_cells`. The
/// "inside" test is "the cell that contains `pos` is one of
/// the cells in the list". Pure grid membership, so a
/// candidate at the cell boundary counts as inside whichever
/// cell the cell-conversion function assigns it to.
pub fn is_inside_gather_zone(pos: Vec2, gather_cells: &[IVec2]) -> bool {
    let cell = world_to_cell(pos);
    gather_cells.contains(&cell)
}

/// True when a Source Stockpile placed at `pos` (with
/// half-footprint `self_radius`) would overlap any of the
/// `obstacles` (each carrying its own center and
/// half-footprint), accounting for the `padding` between
/// footprints. Used to filter out candidates that would
/// clip existing structures or planned structures.
///
/// The check is strictly "centre-to-centre distance is less
/// than the sum of half-footprints plus the padding". A
/// candidate whose edge is exactly at the padding boundary
/// (`distance == sum + padding`) is *not* considered an
/// overlap; only the strict-less case is. The "touch but
/// not overlap" case is intentionally allowed so the
/// builder does not artificially block tight packs.
pub fn overlaps_any_obstacle(
    pos: Vec2,
    self_radius: f32,
    padding: f32,
    obstacles: &[(Vec2, f32)],
) -> bool {
    let extra = self_radius + padding;
    obstacles
        .iter()
        .any(|(center, half_footprint)| pos.distance(*center) < extra + half_footprint)
}

/// Score a candidate for the haul-direction bias. The
/// score is the dot product of the unit vector from the
/// deposit to the candidate with the unit haul direction.
/// Range: `[-1, +1]`. A score of `+1` means the candidate
/// is exactly along the haul direction; `-1` means exactly
/// opposite. When `haul_dir` is the zero vector (no
/// expected haul destination) the score is `0` for every
/// candidate, leaving the choice to the tie-breaker.
pub fn haul_direction_score(candidate_pos: Vec2, deposit_pos: Vec2, haul_dir: Vec2) -> f32 {
    let offset = candidate_pos - deposit_pos;
    let offset_len = offset.length();
    if offset_len < f32::EPSILON {
        return 0.0;
    }
    let haul_len = haul_dir.length();
    if haul_len < f32::EPSILON {
        return 0.0;
    }
    let offset_unit = offset / offset_len;
    let haul_unit = haul_dir / haul_len;
    offset_unit.dot(haul_unit)
}

/// Run the Source Stockpile placement algorithm. Returns
/// the best valid candidate's position, or `None` when
/// every candidate is rejected by the zone-containment or
/// overlap filters. Pure function: same inputs always
/// produce the same answer, so the chosen position is
/// stable across ticks.
///
/// `obstacles` is the union of existing and planned Source
/// Stockpile centers (with their half-footprints) plus any
/// in-tick "newly planned" positions the demand system
/// tracks locally. `haul_direction` is the unit vector
/// toward the expected haul destination (e.g. a Build
/// Zone or the swarm origin); a zero vector means "no
/// bias" and the choice falls back to the lowest
/// `angle_index`.
///
/// On a tie in the haul-direction score, the lower
/// `angle_index` wins. The tie-breaker is deterministic
/// and stable, so two calls with the same inputs pick the
/// same candidate.
#[allow(clippy::too_many_arguments)]
pub fn find_source_stockpile_placement(
    deposit_pos: Vec2,
    gather_cells: &[IVec2],
    obstacles: &[(Vec2, f32)],
    haul_direction: Vec2,
    ring_radius: f32,
    ring_count: usize,
    jitter_amplitude: f32,
    footprint_radius: f32,
    padding: f32,
) -> Option<Vec2> {
    if ring_count == 0 {
        return None;
    }
    let deposit_cell = world_to_cell(deposit_pos);
    let step = std::f32::consts::TAU / ring_count as f32;
    let mut best: Option<(f32, u32, Vec2)> = None;
    for angle_index in 0..ring_count as u32 {
        let angle = angle_index as f32 * step;
        let base = ring_position(deposit_pos, ring_radius, angle);
        let jitter = deterministic_jitter(angle_index, deposit_cell, jitter_amplitude);
        let pos = base + jitter;
        if !is_inside_gather_zone(pos, gather_cells) {
            continue;
        }
        if overlaps_any_obstacle(pos, footprint_radius, padding, obstacles) {
            continue;
        }
        let score = haul_direction_score(pos, deposit_pos, haul_direction);
        let better = match best {
            None => true,
            Some((best_score, best_index, _)) => {
                score > best_score
                    || ((score - best_score).abs() < f32::EPSILON && angle_index < best_index)
            }
        };
        if better {
            best = Some((score, angle_index, pos));
        }
    }
    best.map(|(_, _, pos)| pos)
}

/// Pick a stable, non-overlapping support-structure placement
/// inside one of `build_cells`. Candidate order is deterministic:
/// cells sort by `(x, y)`, then each cell tries its center plus
/// rings of offsets. `kind_seed` lets different structure kinds
/// get different organic-looking ring phases without depending on
/// frame/tick state.
pub fn find_build_zone_placement(
    build_cells: &[IVec2],
    obstacles: &[(Vec2, f32)],
    kind_seed: u32,
) -> Option<(IVec2, Vec2)> {
    let mut cells = build_cells.to_vec();
    cells.sort_by_key(|cell| (cell.x, cell.y));
    let radii = [0.0, 96.0, 160.0, BUILD_ZONE_PLACEMENT_MAX_OFFSET];
    let angles = placement_angles(8);
    for cell in cells {
        let center = crate::ai::get_world_from_zone(cell);
        for (radius_index, radius) in radii.iter().enumerate() {
            if *radius <= 0.0 {
                if !overlaps_any_obstacle(
                    center,
                    BUILDING_FOOTPRINT_RADIUS,
                    BUILDING_FOOTPRINT_PADDING,
                    obstacles,
                ) {
                    return Some((cell, center));
                }
                continue;
            }
            let phase = deterministic_jitter(kind_seed + radius_index as u32, cell, 1.0).x
                * std::f32::consts::TAU;
            for angle in &angles {
                let pos =
                    center + Vec2::new((angle + phase).cos(), (angle + phase).sin()) * *radius;
                if world_to_cell(pos) != cell {
                    continue;
                }
                if overlaps_any_obstacle(
                    pos,
                    BUILDING_FOOTPRINT_RADIUS,
                    BUILDING_FOOTPRINT_PADDING,
                    obstacles,
                ) {
                    continue;
                }
                return Some((cell, pos));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! ("Plan Source Stockpile inside the Gather Zone",
    //! "stable across ticks", "no planning when no valid
    //! candidate exists") are covered by
    //! `tests/behavior/source_stockpile_placement.rs`.

    use super::*;

    const EPS: f32 = 1e-3;

    #[test]
    fn placement_angles_returns_count_evenly_spaced_angles() {
        let angles = placement_angles(8);
        assert_eq!(angles.len(), 8);
        // First angle is east (0 radians).
        assert!(angles[0].abs() < EPS);
        // Spaced by TAU / count = 45 degrees = pi/4 radians.
        let step = std::f32::consts::TAU / 8.0;
        for (i, angle) in angles.iter().enumerate() {
            let expected = i as f32 * step;
            assert!(
                (angle - expected).abs() < EPS,
                "angle {i} must be {expected}, got {angle}"
            );
        }
        // Last angle is one step short of TAU.
        assert!((angles[angles.len() - 1] - (std::f32::consts::TAU - step)).abs() < EPS);
    }

    #[test]
    fn placement_angles_handles_zero_and_one() {
        assert!(placement_angles(0).is_empty());
        let one = placement_angles(1);
        assert_eq!(one.len(), 1);
        assert!(one[0].abs() < EPS);
    }

    #[test]
    fn ring_position_places_candidate_at_radius_on_the_given_angle() {
        let deposit = Vec2::new(10.0, 20.0);
        // East of the deposit.
        let east = ring_position(deposit, 100.0, 0.0);
        assert!((east - Vec2::new(110.0, 20.0)).length() < EPS);
        // North of the deposit.
        let north = ring_position(deposit, 100.0, std::f32::consts::FRAC_PI_2);
        assert!((north - Vec2::new(10.0, 120.0)).length() < EPS);
        // The candidate is exactly at the configured radius
        // from the deposit.
        assert!((east - deposit).length() - 100.0 < EPS);
        assert!((north - deposit).length() - 100.0 < EPS);
    }

    #[test]
    fn deterministic_jitter_is_stable_across_repeated_calls() {
        // Same inputs -> same output. The "stable across
        // ticks" half of the contract is just this
        // property: the function is pure, so repeated
        // calls with the same arguments give the same
        // answer.
        let cell = IVec2::new(3, -2);
        let a1 = deterministic_jitter(2, cell, 16.0);
        let a2 = deterministic_jitter(2, cell, 16.0);
        assert_eq!(a1, a2, "jitter must be a pure function of the inputs");
    }

    #[test]
    fn deterministic_jitter_amplitude_bounds() {
        // The returned offset must be in `[-amplitude, +amplitude]`
        // on both axes for every angle index and cell we might
        // pick. We sweep the space and assert the bound.
        let amplitude = 16.0;
        for cell in [IVec2::new(0, 0), IVec2::new(7, -3), IVec2::new(-12, 5)] {
            for angle_index in 0..32u32 {
                let j = deterministic_jitter(angle_index, cell, amplitude);
                assert!(
                    j.x.abs() <= amplitude + EPS,
                    "jitter.x out of bounds: {} for angle_index={angle_index}, cell={cell:?}",
                    j.x
                );
                assert!(
                    j.y.abs() <= amplitude + EPS,
                    "jitter.y out of bounds: {} for angle_index={angle_index}, cell={cell:?}",
                    j.y
                );
            }
        }
    }

    #[test]
    fn deterministic_jitter_zero_amplitude_means_zero_offset() {
        // A non-positive amplitude collapses to a zero
        // offset. Useful for tests that want the ring
        // positions only.
        let cell = IVec2::new(1, 2);
        assert_eq!(deterministic_jitter(0, cell, 0.0), Vec2::ZERO);
        assert_eq!(deterministic_jitter(5, cell, -1.0), Vec2::ZERO);
    }

    #[test]
    fn deterministic_jitter_varies_with_angle_index_and_cell() {
        // Different angle indices produce different offsets
        // (most of the time) so the ring does not collapse
        // to a single jittered point. The hash mixes the
        // angle index in, so neighbouring indices are
        // guaranteed distinct seeds.
        let cell = IVec2::new(0, 0);
        let a = deterministic_jitter(0, cell, 16.0);
        let b = deterministic_jitter(1, cell, 16.0);
        assert_ne!(a, b, "adjacent angle indices must produce different jitter");
        // Different cells for the same angle index also
        // produce different offsets: the seed includes the
        // cell bits.
        let c1 = deterministic_jitter(0, IVec2::new(0, 0), 16.0);
        let c2 = deterministic_jitter(0, IVec2::new(1, 0), 16.0);
        assert_ne!(c1, c2, "different cells must produce different jitter");
    }

    #[test]
    fn is_inside_gather_zone_accepts_point_in_painted_cell() {
        let cells = vec![IVec2::new(0, 0), IVec2::new(1, 0)];
        // Cell (0, 0) world rect is (0, 0) - (512, 512); the
        // candidate is at the cell center.
        assert!(is_inside_gather_zone(Vec2::new(256.0, 256.0), &cells));
        // Edge of cell (0, 0) is still inside.
        assert!(is_inside_gather_zone(Vec2::new(511.0, 511.0), &cells));
    }

    #[test]
    fn is_inside_gather_zone_rejects_point_in_unpainted_cell() {
        let cells = vec![IVec2::new(0, 0)];
        // Cell (1, 0) world rect is (512, 0) - (1024, 512);
        // the candidate is at that cell's center.
        assert!(!is_inside_gather_zone(Vec2::new(768.0, 256.0), &cells));
    }

    #[test]
    fn is_inside_gather_zone_with_empty_list_rejects_everything() {
        // The "no gather cells" half of the contract: when
        // the swarm has not painted a Gather Zone, the
        // placement must reject every candidate and return
        // None. The single-cell check is the cheapest way
        // to pin that the algorithm does not fall back to
        // a permissive default.
        assert!(!is_inside_gather_zone(Vec2::new(256.0, 256.0), &[]));
    }

    #[test]
    fn overlaps_any_obstacle_centre_collision() {
        // Two Source Stockpiles at the same position. The
        // overlap must be true.
        let obstacles = vec![(Vec2::new(0.0, 0.0), 32.0_f32)];
        assert!(overlaps_any_obstacle(
            Vec2::new(0.0, 0.0),
            32.0,
            0.0,
            &obstacles
        ));
        // Centres 60 apart: inside the 64-sum half-footprint
        // overlap, no padding. Still overlaps.
        assert!(overlaps_any_obstacle(
            Vec2::new(60.0, 0.0),
            32.0,
            0.0,
            &obstacles
        ));
    }

    #[test]
    fn overlaps_any_obstacle_padding_rejects_near_misses() {
        // Centres 70 apart: outside the 64 footprint sum
        // but inside 64 + padding = 80. The overlap is
        // rejected only when padding > 0.
        let obstacles = vec![(Vec2::new(0.0, 0.0), 32.0_f32)];
        assert!(!overlaps_any_obstacle(
            Vec2::new(70.0, 0.0),
            32.0,
            0.0,
            &obstacles
        ));
        // Same geometry, padding 16: now 70 < 80, so the
        // overlap is true.
        assert!(overlaps_any_obstacle(
            Vec2::new(70.0, 0.0),
            32.0,
            16.0,
            &obstacles
        ));
    }

    #[test]
    fn overlaps_any_obstacle_exact_touch_is_allowed() {
        // The check is strict-less so a centre-to-centre
        // distance of exactly the threshold counts as
        // "just touching, not overlapping". This keeps
        // tight packs readable.
        let obstacles = vec![(Vec2::new(0.0, 0.0), 32.0_f32)];
        // Centres 64 apart: 64 == 32 + 32 (no padding).
        // Allowed, not an overlap.
        assert!(!overlaps_any_obstacle(
            Vec2::new(64.0, 0.0),
            32.0,
            0.0,
            &obstacles
        ));
        // Centres 80 apart with padding 16: 80 == 32 + 32
        // + 16. Allowed, not an overlap.
        assert!(!overlaps_any_obstacle(
            Vec2::new(80.0, 0.0),
            32.0,
            16.0,
            &obstacles
        ));
    }

    #[test]
    fn overlaps_any_obstacle_empty_list_accepts_everything() {
        // No obstacles means no overlap. The "no valid
        // placement" path goes through the gather-zone
        // filter, not the obstacle filter, so this case is
        // reachable only when a swarm has no gather paint.
        assert!(!overlaps_any_obstacle(Vec2::new(0.0, 0.0), 32.0, 16.0, &[]));
    }

    #[test]
    fn haul_direction_score_is_dot_product_with_unit_vectors() {
        // Candidate east of deposit, haul direction east.
        let score = haul_direction_score(Vec2::new(10.0, 0.0), Vec2::ZERO, Vec2::new(1.0, 0.0));
        assert!((score - 1.0).abs() < EPS);
        // Candidate east, haul direction west.
        let score = haul_direction_score(Vec2::new(10.0, 0.0), Vec2::ZERO, Vec2::new(-1.0, 0.0));
        assert!((score - -1.0).abs() < EPS);
        // Candidate east, haul direction north.
        let score = haul_direction_score(Vec2::new(10.0, 0.0), Vec2::ZERO, Vec2::new(0.0, 1.0));
        assert!(score.abs() < EPS, "perpendicular score is 0; got {score}");
    }

    #[test]
    fn haul_direction_score_collapses_to_zero_when_direction_unset() {
        // Zero haul direction means "no bias" -- every
        // candidate scores 0 and the algorithm falls back
        // to the angle-index tie-breaker.
        let score = haul_direction_score(Vec2::new(10.0, 0.0), Vec2::ZERO, Vec2::ZERO);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn haul_direction_score_handles_non_unit_haul_direction() {
        // The score uses unit vectors, so a non-unit
        // haul_dir produces the same score as its unit
        // version. The helper is forgiving on the
        // caller's input shape.
        let score = haul_direction_score(Vec2::new(10.0, 0.0), Vec2::ZERO, Vec2::new(7.0, 0.0));
        assert!((score - 1.0).abs() < EPS);
        let score = haul_direction_score(Vec2::new(0.0, 10.0), Vec2::ZERO, Vec2::new(0.0, 0.001));
        assert!((score - 1.0).abs() < EPS);
    }

    #[test]
    fn find_source_stockpile_placement_picks_highest_scored_candidate() {
        // The deposit is at the origin; the haul direction
        // is north; the Gather Zone is the deposit's cell.
        // Eight candidates, the one due north (angle
        // pi/2) should win on score.
        let deposit = Vec2::new(256.0, 256.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        let haul = Vec2::new(0.0, 1.0);
        let chosen = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &[],
            haul,
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        )
        .expect(
            "a candidate must be chosen when the gather cell is painted and there are no obstacles",
        );
        // With jitter = 0, the chosen position is exactly on
        // the north-of-deposit ring position. The ring radius
        // is 96, so the y component is 256 + 96 = 352 and
        // the x component is 256.
        assert!((chosen.x - 256.0).abs() < EPS, "x drift: {chosen}");
        assert!(
            (chosen.y - (256.0 + SOURCE_STOCKPILE_PLACEMENT_RADIUS)).abs() < EPS,
            "y drift: {chosen}"
        );
    }

    #[test]
    fn find_source_stockpile_placement_is_stable_across_calls() {
        // Same inputs -> same answer. The "stable across
        // ticks" half of the contract for the full
        // pipeline.
        let deposit = Vec2::new(100.0, 200.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        let obstacles: Vec<(Vec2, f32)> = vec![(Vec2::new(150.0, 250.0), 32.0)];
        let haul = Vec2::new(1.0, 0.0);
        let a = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &obstacles,
            haul,
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            SOURCE_STOCKPILE_JITTER_AMPLITUDE,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            SOURCE_STOCKPILE_PADDING,
        );
        let b = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &obstacles,
            haul,
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            SOURCE_STOCKPILE_JITTER_AMPLITUDE,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            SOURCE_STOCKPILE_PADDING,
        );
        assert_eq!(a, b, "placement must be a pure function of the inputs");
    }

    #[test]
    fn find_source_stockpile_placement_returns_none_when_no_gather_cells() {
        // The "no Gather Zone" half of the contract: every
        // candidate is rejected by the zone-containment
        // filter, so the algorithm returns None.
        let deposit = Vec2::new(256.0, 256.0);
        let chosen = find_source_stockpile_placement(
            deposit,
            &[],
            &[],
            Vec2::new(1.0, 0.0),
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        );
        assert!(
            chosen.is_none(),
            "no Gather Zone must mean no placement; got {chosen:?}"
        );
    }

    #[test]
    fn find_source_stockpile_placement_returns_none_when_gather_cell_is_wrong() {
        // The candidate ring is around the deposit. If the
        // gather cell is somewhere else, every candidate
        // lands in the deposit's cell (which is not in the
        // gather list) and the algorithm returns None.
        let deposit = Vec2::new(256.0, 256.0);
        let chosen = find_source_stockpile_placement(
            deposit,
            &[IVec2::new(5, 5)],
            &[],
            Vec2::new(1.0, 0.0),
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        );
        assert!(chosen.is_none());
    }

    #[test]
    fn find_source_stockpile_placement_rejects_candidates_blocked_by_obstacles() {
        // The deposit is at the origin; the gather cell is
        // the same; a single obstacle sits at the east-of-
        // deposit ring position. The east candidate is
        // rejected by the overlap test, so the algorithm
        // picks a different valid candidate.
        let deposit = Vec2::new(256.0, 256.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        // East of the deposit at the ring radius.
        let obstacle_pos = Vec2::new(256.0 + SOURCE_STOCKPILE_PLACEMENT_RADIUS, 256.0);
        let obstacles = vec![(obstacle_pos, 32.0_f32)];
        let chosen = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &obstacles,
            Vec2::new(1.0, 0.0),
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        )
        .expect("a non-east candidate must be chosen");
        // The chosen position is not the obstacle's
        // position.
        assert!(
            (chosen - obstacle_pos).length() > 1.0,
            "chosen position must not be the obstacle's position; got {chosen:?}"
        );
    }

    #[test]
    fn find_source_stockpile_placement_returns_none_when_every_candidate_overlaps() {
        // The deposit is at the origin; the gather cell is
        // the same; every ring candidate is blocked by an
        // obstacle. The algorithm must return None rather
        // than pick an overlapping fallback.
        let deposit = Vec2::new(256.0, 256.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        let radius = SOURCE_STOCKPILE_PLACEMENT_RADIUS;
        let count = SOURCE_STOCKPILE_PLACEMENT_COUNT;
        let obstacles: Vec<(Vec2, f32)> = placement_angles(count)
            .into_iter()
            .map(|a| (ring_position(deposit, radius, a), 64.0_f32))
            .collect();
        let chosen = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &obstacles,
            Vec2::new(1.0, 0.0),
            radius,
            count,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        );
        assert!(
            chosen.is_none(),
            "overlap-everywhere must mean no placement; got {chosen:?}"
        );
    }

    #[test]
    fn find_source_stockpile_placement_uses_padding_in_obstacle_rejection() {
        // The deposit is at the origin; the gather cell is
        // the same. A near-miss obstacle is placed at a
        // distance of `footprint + padding - 1` from one
        // candidate. The algorithm must reject the
        // candidate because of the padding.
        let deposit = Vec2::new(256.0, 256.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        let radius = SOURCE_STOCKPILE_PLACEMENT_RADIUS;
        // 90 < 96 (the east-of-deposit ring position is at
        // (256 + 96, 256) = (352, 256)). The obstacle sits
        // 90 units to the east of the ring position. The
        // padding 16 makes the threshold 80, so 90 is
        // inside the padding zone: the candidate is
        // rejected.
        let obstacle_pos = Vec2::new(256.0 + radius + 6.0, 256.0);
        let obstacles = vec![(obstacle_pos, 32.0_f32)];
        let chosen = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &obstacles,
            // Haul direction is east, so the algorithm
            // would prefer the east candidate; the
            // padding test must make the algorithm reject
            // it.
            Vec2::new(1.0, 0.0),
            radius,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            SOURCE_STOCKPILE_PADDING,
        )
        .expect("a non-east candidate must be chosen");
        // The chosen position is not the obstacle's
        // position.
        assert!(
            (chosen - obstacle_pos).length() > 1.0,
            "chosen position must not be the obstacle's position; got {chosen:?}"
        );
    }

    #[test]
    fn find_source_stockpile_placement_score_bias_picks_aligned_candidate() {
        // The deposit is at the origin; the gather cell is
        // the same. With jitter = 0 and a clear haul
        // direction to the east, the algorithm must pick
        // the east candidate (angle 0).
        let deposit = Vec2::new(256.0, 256.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        let chosen = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &[],
            Vec2::new(1.0, 0.0),
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        )
        .expect("a candidate must be chosen");
        // East of the deposit at the ring radius.
        let expected = Vec2::new(256.0 + SOURCE_STOCKPILE_PLACEMENT_RADIUS, 256.0);
        assert!(
            (chosen - expected).length() < EPS,
            "haul direction east must pick the east candidate; got {chosen:?}"
        );
    }

    #[test]
    fn find_source_stockpile_placement_tie_breaks_on_lowest_angle_index() {
        // With a zero haul direction, every candidate
        // scores 0 and the algorithm must pick the
        // candidate with angle_index 0 (east).
        let deposit = Vec2::new(256.0, 256.0);
        let gather_cells = vec![IVec2::new(0, 0)];
        let chosen = find_source_stockpile_placement(
            deposit,
            &gather_cells,
            &[],
            Vec2::ZERO,
            SOURCE_STOCKPILE_PLACEMENT_RADIUS,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            0.0,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            0.0,
        )
        .expect("a candidate must be chosen");
        // East of the deposit at the ring radius.
        let expected = Vec2::new(256.0 + SOURCE_STOCKPILE_PLACEMENT_RADIUS, 256.0);
        assert!(
            (chosen - expected).length() < EPS,
            "zero haul direction must tie-break on angle 0; got {chosen:?}"
        );
    }
}
