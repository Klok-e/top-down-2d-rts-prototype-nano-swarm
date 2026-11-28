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
    cell: Option<IVec2>,
}

/// Start-independent work candidates of one region; see
/// [`InteractionRegion::candidate_parts`].
pub(crate) struct CandidateParts {
    /// Tile-limited approaches, always kept.
    fixed: Vec<Vec2>,
    /// Fine-cell projections, kept unless near an earlier candidate.
    probes: Vec<Vec2>,
    /// Perimeter samples, always kept.
    perimeter: Vec<Vec2>,
}

/// Exact bit identity of an interaction region for hashed cache keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct InteractionBits {
    center: [u32; 2],
    rotation: [u32; 4],
    shape: crate::navigation::ShapeBits,
    cell: Option<IVec2>,
}

impl InteractionRegion {
    pub(crate) fn bits(self) -> InteractionBits {
        InteractionBits {
            center: self.center.to_array().map(f32::to_bits),
            rotation: self.rotation.to_array().map(f32::to_bits),
            shape: self.shape.bits(),
            cell: self.cell,
        }
    }

    /// Deposits declare their physical world-space radius independently of sprite scale.
    pub fn deposit(transform: &Transform, radius: f32) -> Self {
        Self {
            center: transform.translation.truncate(),
            rotation: Quat::IDENTITY,
            shape: Obstacle::deposit(Vec2::ZERO, radius),
            cell: None,
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
            cell: None,
        }
    }

    /// Limit the valid exterior interaction positions to this intent tile.
    pub fn within_cell(mut self, cell: IVec2) -> Self {
        self.cell = Some(cell);
        self
    }

    pub(crate) fn deposit_tile_band(self) -> Option<(Vec2, Vec2, Vec2, [f32; 2])> {
        let cell = self.cell?;
        let Obstacle::Circle { radius, .. } = self.shape else {
            return None;
        };
        let (min, max) = Self::tile_bounds(cell);
        let inner = radius + BODY_RADIUS;
        Some((min, max, self.center, [inner, inner + 2.0 * STOP_THRESHOLD]))
    }

    pub(crate) fn deposit_work_within_range(self, point: Vec2, range: f32) -> bool {
        let Some((min, max, center, radii)) = self.deposit_tile_band() else {
            return false;
        };
        let range_squared = range.powi(2);
        let projected = point.clamp(min, max);
        if projected.distance_squared(point) > range_squared {
            return false;
        }
        if self.contains(projected) {
            return true;
        }

        let direction = (point - center).try_normalize().unwrap_or(Vec2::X);
        if radii.into_iter().any(|radius| {
            let contact = center + direction * radius;
            self.contains(contact) && contact.distance_squared(point) <= range_squared
        }) {
            return true;
        }

        // If the closest radial point is outside the cell, the closest work position
        // is at the intersection of the work band and the cell's edge.
        let mut boundary = Vec::new();
        self.add_tile_boundary_candidates(
            self.cell.expect("a deposit tile band has a cell"),
            &mut boundary,
        );
        boundary
            .into_iter()
            .any(|contact| contact.distance_squared(point) <= range_squared)
    }

    /// True only while the entire body is outside and its surface can reach work.
    pub fn contains(self, position: Vec2) -> bool {
        let (distance, _, _) = self.surface(position);
        self.cell
            .is_none_or(|cell| super::world_to_cell(position) == cell)
            && (BODY_RADIUS - 0.001..=BODY_RADIUS + 2.0 * STOP_THRESHOLD + 0.001)
                .contains(&distance)
    }

    /// Closest exterior work position; an interior start exits through its nearest edge.
    pub fn approach(self, position: Vec2) -> Vec2 {
        let (_, surface, normal) = self.surface(position);
        self.center
            + (self.rotation * (surface + normal * (BODY_RADIUS + STOP_THRESHOLD)).extend(0.0))
                .truncate()
    }

    pub fn movement_from(self, position: Vec2) -> DirectMovementComponent {
        let approach = self.approach(position);
        DirectMovementComponent {
            speed: None,
            xy: self
                .in_cell(approach)
                .or_else(|| self.candidates(position).into_iter().next())
                .unwrap_or(position),
            stop_radius: STOP_THRESHOLD,
            interaction: Some(self),
        }
    }

    /// Exterior approaches projected from fine cells around the whole footprint.
    /// Include the closest point without requiring a work position to coincide
    /// with a cell center in the thin interaction band.
    pub(crate) fn candidates(self, start: Vec2) -> Vec<Vec2> {
        self.candidates_from(&self.approach_parts(), start)
    }

    /// The parts of [`Self::work_candidates_from`] that do not depend on the
    /// start, in the order it considers them.
    pub(crate) fn candidate_parts(self) -> CandidateParts {
        CandidateParts {
            perimeter: self.perimeter_candidates(),
            ..self.approach_parts()
        }
    }

    /// [`Self::candidate_parts`] without the perimeter samples.
    pub(crate) fn approach_parts(self) -> CandidateParts {
        let mut fixed = Vec::new();
        if let Some(cell) = self.cell {
            let center = crate::ai::get_world_from_zone(cell);
            fixed.extend(self.in_cell(self.approach(center)));
            self.add_tile_boundary_candidates(cell, &mut fixed);
        }
        let (min, max) = self.candidate_cell_bounds();
        let mut probes = Vec::new();
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let point = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
                let distance = self.surface(point).0;
                if (0.0..=BODY_RADIUS + CELL_WIDTH * 1.5).contains(&distance) {
                    probes.extend(self.in_cell(self.approach(point)));
                }
            }
        }
        CandidateParts {
            fixed,
            probes,
            perimeter: Vec::new(),
        }
    }

    /// [`Self::candidates`] from parts this region computed earlier.
    pub(crate) fn candidates_from(self, parts: &CandidateParts, start: Vec2) -> Vec<Vec2> {
        let mut candidates = Vec::new();
        candidates.extend(self.in_cell(self.approach(start)));
        candidates.extend_from_slice(&parts.fixed);
        for &candidate in &parts.probes {
            if candidates
                .iter()
                .all(|old| old.distance_squared(candidate) > 0.01)
            {
                candidates.push(candidate);
            }
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        candidates
    }

    pub(crate) fn candidate_cell_bounds(self) -> (IVec2, IVec2) {
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
        (min, max)
    }

    /// Candidates plus closely spaced perimeter probes for finding gaps between
    /// physical working bodies, from parts this region computed earlier.
    pub(crate) fn work_candidates_from(self, parts: &CandidateParts, start: Vec2) -> Vec<Vec2> {
        let mut candidates = self.candidates_from(parts, start);
        candidates.extend_from_slice(&parts.perimeter);
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        candidates
    }

    fn perimeter_candidates(self) -> Vec<Vec2> {
        // Sample the whole perimeter more finely than a body, including face centers.
        // Fine-grid projections alone can miss the only gap between two working bodies.
        let samples = match self.shape {
            Obstacle::Circle { radius, .. } => {
                let count = ((std::f32::consts::TAU * (radius + BODY_RADIUS) / (BODY_RADIUS / 2.0))
                    .ceil() as usize)
                    .max(8);
                (0..count)
                    .map(|i| {
                        Vec2::from_angle(std::f32::consts::TAU * i as f32 / count as f32)
                            * (radius + BODY_RADIUS)
                    })
                    .collect::<Vec<_>>()
            }
            Obstacle::Rectangle { half, .. } => {
                let outer = half + Vec2::splat(BODY_RADIUS + STOP_THRESHOLD);
                let mut samples = Vec::new();
                for axis in 0..2 {
                    let along = if axis == 0 { outer.y } else { outer.x };
                    let steps = ((2.0 * along / (BODY_RADIUS / 2.0)).ceil() as usize)
                        .max(2)
                        .next_multiple_of(2);
                    for i in 0..=steps {
                        let offset = -along + 2.0 * along * i as f32 / steps as f32;
                        for sign in [-1.0, 1.0] {
                            samples.push(if axis == 0 {
                                Vec2::new(sign * outer.x, offset)
                            } else {
                                Vec2::new(offset, sign * outer.y)
                            });
                        }
                    }
                }
                samples
            }
        };
        samples
            .into_iter()
            .filter_map(|local| {
                self.in_cell(
                    self.approach(self.center + (self.rotation * local.extend(0.0)).truncate()),
                )
            })
            .collect()
    }

    fn in_cell(self, point: Vec2) -> Option<Vec2> {
        let Some(cell) = self.cell else {
            return Some(point);
        };
        let (min, max) = Self::tile_bounds(cell);
        let point = point.clamp(min, max);
        self.contains(point).then_some(point)
    }

    fn tile_bounds(cell: IVec2) -> (Vec2, Vec2) {
        let min = cell.as_vec2() * crate::ZONE_BLOCK_SIZE;
        let exclusive_max = (cell + IVec2::ONE).as_vec2();
        let max = Vec2::new(exclusive_max.x.next_down(), exclusive_max.y.next_down())
            * crate::ZONE_BLOCK_SIZE;
        (min, max)
    }

    fn add_tile_boundary_candidates(self, cell: IVec2, candidates: &mut Vec<Vec2>) {
        let (min, max) = Self::tile_bounds(cell);
        candidates.extend(
            [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)]
                .into_iter()
                .filter_map(|point| self.in_cell(point)),
        );
        let Obstacle::Circle { radius, .. } = self.shape else {
            return;
        };
        // A thin arc in a tile corner can lie between all projected fine-grid sample points.
        for clearance in [0.0, STOP_THRESHOLD, 2.0 * STOP_THRESHOLD] {
            let contact = radius + BODY_RADIUS + clearance;
            for axis in 0..2 {
                let other_axis = 1 - axis;
                for edge in [min[axis], max[axis]] {
                    let offset = edge - self.center[axis];
                    let cross = contact * contact - offset * offset;
                    if cross < 0.0 {
                        continue;
                    }
                    for other_offset in [-cross.sqrt(), cross.sqrt()] {
                        let mut point = self.center;
                        point[axis] = edge;
                        point[other_axis] += other_offset;
                        if point.cmpge(min).all()
                            && point.cmple(max).all()
                            && let Some(point) = self.in_cell(point)
                        {
                            candidates.push(point);
                        }
                    }
                }
            }
        }
    }

    fn surface(self, position: Vec2) -> (f32, Vec2, Vec2) {
        let local = (self.rotation.inverse() * (position - self.center).extend(0.0)).truncate();
        self.shape.surface(local)
    }
}

#[cfg(test)]
mod tests {
    use rand::{RngExt, SeedableRng, rngs::StdRng};

    use super::*;

    /// The candidate search before its start-independent parts were split out.
    fn reference_candidates(region: InteractionRegion, start: Vec2) -> Vec<Vec2> {
        let this = region;
        let (min, max) = this.candidate_cell_bounds();
        let mut candidates = Vec::new();
        candidates.extend(this.in_cell(this.approach(start)));
        if let Some(cell) = this.cell {
            let center = crate::ai::get_world_from_zone(cell);
            candidates.extend(this.in_cell(this.approach(center)));
            this.add_tile_boundary_candidates(cell, &mut candidates);
        }
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let point = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
                let distance = this.surface(point).0;
                if (0.0..=BODY_RADIUS + CELL_WIDTH * 1.5).contains(&distance)
                    && let Some(candidate) = this.in_cell(this.approach(point))
                    && candidates
                        .iter()
                        .all(|old| old.distance_squared(candidate) > 0.01)
                {
                    candidates.push(candidate);
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        candidates
    }

    fn reference_work_candidates(region: InteractionRegion, start: Vec2) -> Vec<Vec2> {
        let this = region;
        let mut candidates = reference_candidates(this, start);
        // Sample the whole perimeter more finely than a body, including face centers.
        // Fine-grid projections alone can miss the only gap between two working bodies.
        let samples = match this.shape {
            Obstacle::Circle { radius, .. } => {
                let count = ((std::f32::consts::TAU * (radius + BODY_RADIUS) / (BODY_RADIUS / 2.0))
                    .ceil() as usize)
                    .max(8);
                (0..count)
                    .map(|i| {
                        Vec2::from_angle(std::f32::consts::TAU * i as f32 / count as f32)
                            * (radius + BODY_RADIUS)
                    })
                    .collect::<Vec<_>>()
            }
            Obstacle::Rectangle { half, .. } => {
                let outer = half + Vec2::splat(BODY_RADIUS + STOP_THRESHOLD);
                let mut samples = Vec::new();
                for axis in 0..2 {
                    let along = if axis == 0 { outer.y } else { outer.x };
                    let steps = ((2.0 * along / (BODY_RADIUS / 2.0)).ceil() as usize)
                        .max(2)
                        .next_multiple_of(2);
                    for i in 0..=steps {
                        let offset = -along + 2.0 * along * i as f32 / steps as f32;
                        for sign in [-1.0, 1.0] {
                            samples.push(if axis == 0 {
                                Vec2::new(sign * outer.x, offset)
                            } else {
                                Vec2::new(offset, sign * outer.y)
                            });
                        }
                    }
                }
                samples
            }
        };
        candidates.extend(samples.into_iter().filter_map(|local| {
            this.in_cell(
                this.approach(this.center + (this.rotation * local.extend(0.0)).truncate()),
            )
        }));
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        candidates
    }

    #[test]
    fn cached_parts_give_the_same_work_candidates_from_any_start() {
        let mut rng = StdRng::seed_from_u64(89);
        for _ in 0..300 {
            let center = Vec2::new(
                rng.random_range(-500.0..500.0),
                rng.random_range(-500.0..500.0),
            );
            let mut transform = Transform::from_translation(center.extend(0.0));
            let region = if rng.random_bool(0.5) {
                InteractionRegion::deposit(&transform, rng.random_range(8.0..90.0))
            } else {
                transform.scale =
                    Vec3::new(rng.random_range(0.5..2.5), rng.random_range(0.5..2.5), 1.0);
                transform.rotation =
                    Quat::from_rotation_z(rng.random_range(0.0..std::f32::consts::TAU));
                InteractionRegion::structure(&transform)
            };
            let region = if rng.random_bool(0.5) {
                region.within_cell(super::super::world_to_cell(center))
            } else {
                region
            };
            let parts = region.candidate_parts();
            for _ in 0..10 {
                let start = center
                    + Vec2::new(
                        rng.random_range(-300.0..300.0),
                        rng.random_range(-300.0..300.0),
                    );
                assert_eq!(
                    region.work_candidates_from(&parts, start),
                    reference_work_candidates(region, start)
                );
                assert_eq!(
                    region.candidates(start),
                    reference_candidates(region, start)
                );
            }
        }
    }
}
