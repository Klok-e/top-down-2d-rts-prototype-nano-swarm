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

impl InteractionRegion {
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
        let (min, max) = self.candidate_cell_bounds();
        let mut candidates = Vec::new();
        candidates.extend(self.in_cell(self.approach(start)));
        if let Some(cell) = self.cell {
            let center = crate::ai::get_world_from_zone(cell);
            candidates.extend(self.in_cell(self.approach(center)));
            self.add_tile_boundary_candidates(cell, &mut candidates);
        }
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let point = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
                let distance = self.surface(point).0;
                if (0.0..=BODY_RADIUS + CELL_WIDTH * 1.5).contains(&distance)
                    && let Some(candidate) = self.in_cell(self.approach(point))
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

    /// Closely spaced perimeter probes for finding gaps between physical working bodies.
    pub(crate) fn work_candidates(self, start: Vec2) -> Vec<Vec2> {
        let mut candidates = self.candidates(start);
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
        candidates.extend(samples.into_iter().filter_map(|local| {
            self.in_cell(
                self.approach(self.center + (self.rotation * local.extend(0.0)).truncate()),
            )
        }));
        candidates.sort_by(|a, b| {
            a.distance_squared(start)
                .total_cmp(&b.distance_squared(start))
        });
        candidates
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
