//! Fine-grained lookup over the Defenders indexed by an engagement pass.
//!
//! Every query answers exactly as a scan of the coarse engagement buckets
//! would: a Defender is a candidate when its coarse bucket lies within the
//! query's bucket radius and its squared distance passes the same `f32` test.
//! The fine grid only narrows which Defenders are tested.

use std::{cmp::Ordering, ops::ControlFlow};

use bevy::{math::DVec2, prelude::*};

use super::EngagementDefender;
use crate::{
    nanobot::SwarmId,
    spatial::{ceil_f64, floor_f64},
};

/// Fine cell side in world units, before widening for spread-out layouts.
const FINE_CELL_SIZE: f64 = 64.0;
/// Fixed-point scale of the strength totals: exact integer sums of rounded strengths.
const STRENGTH_SCALE: f64 = (1_u64 << 32) as f64;
/// Largest strength the fixed-point totals hold without overflow.
const MAX_TOTALED_STRENGTH: f64 = 256.0;
/// Upper bound on fine cells, so spread-out layouts widen cells instead of allocating.
const MAX_FINE_CELLS: f64 = (1 << 20) as f64;
/// Fine cells allowed per Defender, so a few far-apart Defenders get a small grid.
const FINE_CELLS_PER_DEFENDER: f64 = 16.0;
/// Fine cells always allowed, however few Defenders there are.
const MIN_FINE_CELL_BUDGET: f64 = (1 << 12) as f64;

#[derive(Clone, Copy)]
struct Candidate {
    position: Vec2,
    coarse: IVec2,
    /// Position in insertion order, which ties break by.
    order: u32,
    swarm: SwarmId,
    routing: bool,
    strength: f64,
    key: u64,
}

#[derive(Clone)]
pub(super) struct DefenderIndex {
    coarse_size: f32,
    defenders: Vec<EngagementDefender>,
    /// Every strength is finite and non-negative, so sums in any order bound each other.
    bounded_strengths: bool,
    cell_size: f64,
    /// World position of the first fine cell's corner.
    origin: DVec2,
    width: i64,
    height: i64,
    /// Fine-cell ranges into `cells`, in row-major order.
    starts: Vec<u32>,
    cells: Vec<Candidate>,
    /// Defenders whose position has no fine cell; every query tests them.
    unplaced: Vec<Candidate>,
    /// Running strength totals over `cells`, when every strength fits the fixed-point scale.
    totals: Option<StrengthTotals>,
}

/// Prefix sums over the fine cells, so a run of cells has exact totals in O(1).
#[derive(Clone)]
struct StrengthTotals {
    swarms: Vec<SwarmId>,
    /// Per position in `cells`: every swarm's strength, then per swarm its strength,
    /// its non-routing strength, and its count of positive strengths.
    all: Vec<i64>,
    positive: Vec<u32>,
    by_swarm: Vec<[i64; 2]>,
    positive_by_swarm: Vec<u32>,
}

/// Fixed-point totals of one swarm against the rest, over some Defenders.
#[derive(Clone, Copy, Default)]
struct SideTotals {
    friendly: i64,
    hostile: i64,
    hostile_positive: u64,
    count: u64,
}

impl StrengthTotals {
    fn new(cells: &[Candidate]) -> Self {
        let mut swarms: Vec<_> = cells.iter().map(|candidate| candidate.swarm).collect();
        swarms.sort_unstable();
        swarms.dedup();
        let stride = swarms.len();
        let mut totals = Self {
            all: vec![0; cells.len() + 1],
            positive: vec![0; cells.len() + 1],
            by_swarm: vec![[0; 2]; (cells.len() + 1) * stride],
            positive_by_swarm: vec![0; (cells.len() + 1) * stride],
            swarms,
        };
        for (position, candidate) in cells.iter().enumerate() {
            let strength = fixed_strength(candidate.strength);
            let positive = u32::from(candidate.strength > 0.0);
            totals.all[position + 1] = totals.all[position] + strength;
            totals.positive[position + 1] = totals.positive[position] + positive;
            let swarm = totals.swarm_index(candidate.swarm).unwrap();
            for index in 0..stride {
                let [all, active] = totals.by_swarm[position * stride + index];
                let count = totals.positive_by_swarm[position * stride + index];
                let own = index == swarm;
                totals.by_swarm[(position + 1) * stride + index] = [
                    all + if own { strength } else { 0 },
                    active
                        + if own && !candidate.routing {
                            strength
                        } else {
                            0
                        },
                ];
                totals.positive_by_swarm[(position + 1) * stride + index] =
                    count + if own { positive } else { 0 };
            }
        }
        totals
    }

    fn swarm_index(&self, swarm: SwarmId) -> Option<usize> {
        self.swarms.binary_search(&swarm).ok()
    }

    /// Totals over `cells[start..end]` for `swarm` (at `index`, when present).
    fn side(&self, index: Option<usize>, start: usize, end: usize) -> SideTotals {
        let all = self.all[end] - self.all[start];
        let positive = u64::from(self.positive[end] - self.positive[start]);
        let (own_all, own_active, own_positive) = index.map_or((0, 0, 0), |index| {
            let stride = self.swarms.len();
            let [end_all, end_active] = self.by_swarm[end * stride + index];
            let [start_all, start_active] = self.by_swarm[start * stride + index];
            let count = self.positive_by_swarm[end * stride + index]
                - self.positive_by_swarm[start * stride + index];
            (
                end_all - start_all,
                end_active - start_active,
                u64::from(count),
            )
        });
        SideTotals {
            friendly: own_active,
            hostile: all - own_all,
            hostile_positive: positive - own_positive,
            count: (end - start) as u64,
        }
    }
}

impl std::ops::AddAssign for SideTotals {
    fn add_assign(&mut self, other: Self) {
        self.friendly += other.friendly;
        self.hostile += other.hostile;
        self.hostile_positive += other.hostile_positive;
        self.count += other.count;
    }
}

fn fixed_strength(strength: f64) -> i64 {
    (strength * STRENGTH_SCALE).round() as i64
}

impl DefenderIndex {
    /// Index Defenders in their tie-breaking order, with coarse buckets of `coarse_size`.
    pub(super) fn new(defenders: Vec<EngagementDefender>, coarse_size: f32) -> Self {
        let bounded_strengths = defenders
            .iter()
            .all(|defender| defender.strength.is_finite() && defender.strength >= 0.0);
        let (min, max) = defenders
            .iter()
            .map(|defender| defender.position.as_dvec2())
            .filter(|position| position.is_finite())
            .fold(
                (DVec2::INFINITY, DVec2::NEG_INFINITY),
                |(min, max), position| (min.min(position), max.max(position)),
            );
        // Cell size never changes answers, only how much each query reads.
        let cell_budget = (defenders.len() as f64 * FINE_CELLS_PER_DEFENDER)
            .clamp(MIN_FINE_CELL_BUDGET, MAX_FINE_CELLS);
        let mut cell_size = FINE_CELL_SIZE;
        let (width, height) = loop {
            if min.x > max.x {
                break (0, 0);
            }
            let cells = ((max - min) / cell_size).floor() + 1.0;
            if cells.x * cells.y <= cell_budget {
                break (cells.x as i64, cells.y as i64);
            }
            cell_size *= 2.0;
        };
        let mut index = Self {
            coarse_size,
            defenders: Vec::new(),
            bounded_strengths,
            cell_size,
            origin: min,
            width,
            height,
            starts: vec![0; (width * height + 1) as usize],
            cells: Vec::new(),
            unplaced: Vec::new(),
            totals: None,
        };
        let candidates = defenders
            .iter()
            .enumerate()
            .map(|(order, defender)| {
                let candidate = Candidate {
                    position: defender.position,
                    coarse: coarse_bucket(defender.position, coarse_size),
                    order: order as u32,
                    swarm: defender.swarm,
                    routing: defender.routing,
                    strength: defender.strength,
                    key: defender.key,
                };
                (index.cell_of(defender.position), candidate)
            })
            .collect::<Vec<_>>();
        for (cell, _) in &candidates {
            if let Some(cell) = cell {
                index.starts[cell + 1] += 1;
            }
        }
        for cell in 0..index.starts.len() - 1 {
            index.starts[cell + 1] += index.starts[cell];
        }
        let mut next = index.starts.clone();
        let filler = candidates
            .iter()
            .find_map(|(cell, candidate)| cell.map(|_| *candidate));
        index.cells = filler.map_or_else(Vec::new, |filler| {
            vec![filler; *index.starts.last().unwrap() as usize]
        });
        for (cell, candidate) in candidates {
            match cell {
                Some(cell) => {
                    index.cells[next[cell] as usize] = candidate;
                    next[cell] += 1;
                }
                None => index.unplaced.push(candidate),
            }
        }
        index.defenders = defenders;
        let totaled = index.bounded_strengths
            && index.cells.len() < 1 << 22
            && index
                .cells
                .iter()
                .all(|candidate| candidate.strength <= MAX_TOTALED_STRENGTH);
        index.totals = totaled.then(|| StrengthTotals::new(&index.cells));
        index
    }

    pub(super) fn coarse_bucket(&self, position: Vec2) -> IVec2 {
        coarse_bucket(position, self.coarse_size)
    }

    fn coarse_radius(&self, radius: f32) -> i32 {
        (radius / self.coarse_size).ceil() as i32
    }

    /// The indexed Defenders in insertion order.
    pub(super) fn defenders(&self) -> &[EngagementDefender] {
        &self.defenders
    }

    fn defender(&self, candidate: &Candidate) -> &EngagementDefender {
        &self.defenders[candidate.order as usize]
    }

    fn cell_coordinates(&self, position: Vec2) -> Option<[i64; 2]> {
        let cell = (position.as_dvec2() - self.origin) / self.cell_size;
        let cell = DVec2::new(floor_f64(cell.x), floor_f64(cell.y));
        (cell.is_finite() && (0.0..self.width as f64).contains(&cell.x))
            .then_some(cell)
            .filter(|cell| (0.0..self.height as f64).contains(&cell.y))
            .map(|cell| [cell.x as i64, cell.y as i64])
    }

    fn cell_of(&self, position: Vec2) -> Option<usize> {
        let [x, y] = self.cell_coordinates(position)?;
        Some((y * self.width + x) as usize)
    }

    /// Fine-cell row `y`, from column `first` through `last`.
    fn row(&self, y: i64, first: i64, last: i64) -> &[Candidate] {
        let row = y * self.width;
        let start = self.starts[(row + first) as usize] as usize;
        let end = self.starts[(row + last + 1) as usize] as usize;
        &self.cells[start..end]
    }

    /// Runs of Defenders that together hold every Defender within `radius`.
    fn slices_near(&self, position: Vec2, radius: f32) -> impl Iterator<Item = &[Candidate]> {
        // Widen past the f32 distance test's rounding and the cells' floor rounding.
        let reach = f64::from(radius.abs()) * (1.0 + 1e-5) + 1.0;
        let center = (position.as_dvec2() - self.origin) / self.cell_size;
        let reach_cells = reach / self.cell_size;
        let bounded = center.is_finite() && reach_cells.is_finite();
        let last_row = self.height - 1;
        let (first_row, last_row) = if bounded {
            (
                floor_f64(center.y - reach_cells).max(0.0) as i64,
                floor_f64(center.y + reach_cells).min(last_row as f64) as i64,
            )
        } else {
            (0, last_row)
        };
        let rows = (first_row..=last_row).filter_map(move |y| {
            let (first, last) = if bounded {
                let gap = (y as f64 - center.y)
                    .max(center.y - (y + 1) as f64)
                    .max(0.0);
                let half_width = (reach_cells * reach_cells - gap * gap).max(0.0).sqrt();
                (
                    floor_f64(center.x - half_width).max(0.0) as i64,
                    floor_f64(center.x + half_width).min((self.width - 1) as f64) as i64,
                )
            } else {
                (0, self.width - 1)
            };
            (first <= last).then(|| self.row(y, first, last))
        });
        std::iter::once(self.unplaced.as_slice()).chain(rows)
    }

    /// Visit a superset of the Defenders within `radius`, stopping when `visit` breaks.
    fn visit_near<B>(
        &self,
        position: Vec2,
        radius: f32,
        mut visit: impl FnMut(&Candidate) -> ControlFlow<B>,
    ) -> Option<B> {
        for candidate in self.slices_near(position, radius).flatten() {
            if let ControlFlow::Break(value) = visit(candidate) {
                return Some(value);
            }
        }
        None
    }

    /// Defenders a coarse scan of `coarse_radius` buckets would keep within `radius`.
    fn within(
        &self,
        position: Vec2,
        radius: f32,
        coarse_radius: i32,
    ) -> impl Fn(&Candidate) -> bool {
        let center = self.coarse_bucket(position);
        let coarse_radius = i64::from(coarse_radius.max(0));
        let radius_squared = radius * radius;
        move |candidate| {
            (i64::from(candidate.coarse.x) - i64::from(center.x)).abs() <= coarse_radius
                && (i64::from(candidate.coarse.y) - i64::from(center.y)).abs() <= coarse_radius
                && position.distance_squared(candidate.position) <= radius_squared
        }
    }

    /// Whether any Defender outside `swarm` is within `radius`.
    pub(super) fn any_hostile(&self, position: Vec2, swarm: SwarmId, radius: f32) -> bool {
        let within = self.within(position, radius, self.coarse_radius(radius));
        self.visit_near(position, radius, |candidate| {
            if candidate.swarm != swarm && within(candidate) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_some()
    }

    /// The closest Defender outside `swarm` that a coarse scan of `coarse_radius`
    /// buckets keeps within `radius`, with its squared distance. Equal distances
    /// break by combat key, then by the coarse scan's visiting order.
    pub(super) fn nearest_hostile(
        &self,
        position: Vec2,
        swarm: SwarmId,
        radius: f32,
        coarse_radius: Option<i32>,
    ) -> Option<(f32, &EngagementDefender)> {
        let within = self.within(
            position,
            radius,
            coarse_radius.unwrap_or_else(|| self.coarse_radius(radius)),
        );
        let mut best: Option<(f32, Candidate)> = None;
        let consider = |best: &mut Option<(f32, Candidate)>, candidate: &Candidate| {
            if candidate.swarm == swarm || !within(candidate) {
                return;
            }
            let distance = position.distance_squared(candidate.position);
            let closer = best.is_none_or(|(best_distance, best_candidate)| {
                distance
                    .total_cmp(&best_distance)
                    .then_with(|| candidate.key.cmp(&best_candidate.key))
                    .then_with(|| coarse_order(candidate, &best_candidate))
                    .is_lt()
            });
            if closer {
                *best = Some((distance, *candidate));
            }
        };
        let Some([x, y]) = self.cell_coordinates(position) else {
            self.visit_near(position, radius, |candidate| {
                consider(&mut best, candidate);
                ControlFlow::<()>::Continue(())
            });
            return best.map(|(distance, candidate)| (distance, self.defender(&candidate)));
        };
        for candidate in &self.unplaced {
            consider(&mut best, candidate);
        }
        let reach = f64::from(radius.abs()) * (1.0 + 1e-5) + 1.0;
        // Search rings of cells outward until every unvisited cell is provably farther.
        for ring in 0_i64.. {
            let (first_x, last_x) = (x - ring, x + ring);
            let (first_y, last_y) = (y - ring, y + ring);
            if first_x < 0 && first_y < 0 && last_x >= self.width && last_y >= self.height {
                break;
            }
            let (clamped_first, clamped_last) = (first_x.max(0), last_x.min(self.width - 1));
            for row in first_y.max(0)..=last_y.min(self.height - 1) {
                if row == first_y || row == last_y {
                    for candidate in self.row(row, clamped_first, clamped_last) {
                        consider(&mut best, candidate);
                    }
                    continue;
                }
                if first_x >= 0 {
                    for candidate in self.row(row, first_x, first_x) {
                        consider(&mut best, candidate);
                    }
                }
                if last_x < self.width {
                    for candidate in self.row(row, last_x, last_x) {
                        consider(&mut best, candidate);
                    }
                }
            }
            // Cells beyond this ring are at least `ring` whole cells from the query.
            let clearance = (ring as f64 * self.cell_size - 1.0) * (1.0 - 1e-5);
            if clearance > reach
                || best.is_some_and(|(distance, _)| {
                    clearance > 0.0 && f64::from(distance) < clearance * clearance
                })
            {
                break;
            }
        }
        best.map(|(distance, candidate)| (distance, self.defender(&candidate)))
    }

    /// Whether the strength within `radius` leaves `swarm` outnumbered: hostile
    /// strength is positive and friendly strength, excluding routing Defenders,
    /// falls below `ratio` of it, both summed in the coarse scan's visiting order.
    pub(super) fn outnumbered(
        &self,
        position: Vec2,
        swarm: SwarmId,
        radius: f32,
        ratio: f32,
    ) -> bool {
        let within = self.within(position, radius, self.coarse_radius(radius));
        if self.bounded_strengths {
            // Distance alone decides membership when the query's coarse buckets hold its disk.
            let coarse_radius = self.coarse_radius(radius).max(0);
            let center = self.coarse_bucket(position);
            let reach = radius.abs() * (1.0 + 1e-5) + 1.0;
            let (low, high) = (
                self.coarse_bucket(position - reach),
                self.coarse_bucket(position + reach),
            );
            let held = |bucket: IVec2, sign: i64| {
                [(bucket.x, center.x), (bucket.y, center.y)]
                    .into_iter()
                    .all(|(bucket, center)| {
                        sign * (i64::from(bucket) - i64::from(center)) <= i64::from(coarse_radius)
                    })
            };
            // Far from the origin, f32 rounding could exceed the reach's margin.
            let implied =
                position.abs().max_element() < 1e6 && reach < 1e6 && held(low, -1) && held(high, 1);
            if implied
                && let Some(outnumbered) = self.bounded_by_totals(position, swarm, radius, ratio)
            {
                return outnumbered;
            }
            let radius_squared = radius * radius;
            let (mut friendly, mut hostile, mut count) = (0.0_f64, 0.0_f64, 0_u64);
            for (slice_index, slice) in self.slices_near(position, radius).enumerate() {
                // The unplaced Defenders, first, always get the full test.
                if implied && slice_index > 0 {
                    // Multiplying by 0 or 1 keeps the loop free of unpredictable branches.
                    for candidate in slice {
                        let inside =
                            position.distance_squared(candidate.position) <= radius_squared;
                        let own = candidate.swarm == swarm;
                        let active = inside & own & !candidate.routing;
                        friendly += f64::from(u8::from(active)) * candidate.strength;
                        hostile += f64::from(u8::from(inside & !own)) * candidate.strength;
                        count += u64::from(inside);
                    }
                } else {
                    for candidate in slice.iter().filter(|candidate| within(candidate)) {
                        if candidate.swarm != swarm {
                            hostile += candidate.strength;
                        } else if !candidate.routing {
                            friendly += candidate.strength;
                        }
                        count += 1;
                    }
                }
            }
            if let Some(outnumbered) = bounded_outnumbered(friendly, hostile, count, ratio) {
                return outnumbered;
            }
        }
        // Too close to call: sum exactly as the coarse scan orders Defenders.
        let mut matches = Vec::new();
        self.visit_near(position, radius, |candidate| {
            if within(candidate) {
                matches.push(*candidate);
            }
            ControlFlow::<()>::Continue(())
        });
        matches.sort_unstable_by(coarse_order);
        let (mut friendly, mut hostile) = (0.0, 0.0);
        for candidate in matches {
            if candidate.swarm != swarm {
                hostile += candidate.strength;
            } else if !candidate.routing {
                friendly += candidate.strength;
            }
        }
        hostile > 0.0 && ((friendly / hostile) as f32) < ratio
    }

    /// Decide [`Self::outnumbered`] from the strength totals of whole cells: those
    /// inside the disk bound the sums from below, those it touches from above.
    /// `None` when the bounds do not settle the answer.
    fn bounded_by_totals(
        &self,
        position: Vec2,
        swarm: SwarmId,
        radius: f32,
        ratio: f32,
    ) -> Option<bool> {
        let totals = self.totals.as_ref()?;
        if !self.unplaced.is_empty() || self.width == 0 {
            return None;
        }
        let own = totals.swarm_index(swarm);
        let center = (position.as_dvec2() - self.origin) / self.cell_size;
        let reach = (f64::from(radius.abs()) * (1.0 + 1e-5) + 1.0) / self.cell_size;
        // Every point of a cell within this radius passes the f32 distance test.
        let inside = (f64::from(radius.abs()) * (1.0 - 1e-5) - 1.0) / self.cell_size;
        if !(center.is_finite() && reach.is_finite()) {
            return None;
        }
        let first_row = floor_f64(center.y - reach).max(0.0) as i64;
        let last_row = floor_f64(center.y + reach).min((self.height - 1) as f64) as i64;
        let (mut inner, mut outer) = (SideTotals::default(), SideTotals::default());
        for y in first_row..=last_row {
            let row = (y * self.width) as usize;
            let span = |first: i64, last: i64| {
                totals.side(
                    own,
                    self.starts[row + first as usize] as usize,
                    self.starts[row + last as usize + 1] as usize,
                )
            };
            let near = (y as f64 - center.y)
                .max(center.y - (y + 1) as f64)
                .max(0.0);
            let half_width = (reach * reach - near * near).max(0.0).sqrt();
            let first = floor_f64(center.x - half_width).max(0.0) as i64;
            let last = floor_f64(center.x + half_width).min((self.width - 1) as f64) as i64;
            if first > last {
                continue;
            }
            outer += span(first, last);
            let far = (y as f64 - center.y)
                .abs()
                .max(((y + 1) as f64 - center.y).abs());
            if far < inside {
                let half_width = (inside * inside - far * far).sqrt();
                let first_inside = (ceil_f64(center.x - half_width) as i64).max(first);
                let last_inside = (floor_f64(center.x + half_width) as i64 - 1).min(last);
                if first_inside <= last_inside {
                    inner += span(first_inside, last_inside);
                }
            }
        }
        // A sum of non-negative terms is positive exactly when one term is.
        if outer.hostile_positive == 0 {
            return Some(false);
        }
        if inner.hostile_positive == 0 {
            return None;
        }
        // Fixed-point sums are exact sums of strengths rounded by half a unit each, and
        // a sum in the coarse scan's order is within `delta` of the exact sum.
        let error = (outer.count + 1) as f64 / STRENGTH_SCALE;
        let delta = 8.0 * (outer.count as f64 + 2.0) * f64::EPSILON;
        if delta >= 0.5 {
            return None;
        }
        let real = |total: i64| total as f64 / STRENGTH_SCALE;
        let lowest =
            (real(inner.friendly) - error).max(0.0) / (real(outer.hostile) + error) * (1.0 - delta);
        let least_hostile = real(inner.hostile) - error;
        let highest = if least_hostile > 0.0 {
            (real(outer.friendly) + error) / least_hostile * (1.0 + delta)
        } else {
            f64::INFINITY
        };
        if (highest as f32) < ratio {
            Some(true)
        } else if (lowest as f32) >= ratio {
            Some(false)
        } else {
            None
        }
    }

    /// Other Defenders of `defender`'s swarm within `radius`, on the positive and
    /// negative side of `tangent`, from a coarse scan of one bucket around it.
    pub(super) fn friendly_sides(
        &self,
        defender: &EngagementDefender,
        radius: f32,
        tangent: Vec2,
    ) -> (usize, usize) {
        let within = self.within(defender.position, radius, 1);
        let (mut positive, mut negative) = (0, 0);
        self.visit_near(defender.position, radius, |candidate| {
            if candidate.swarm == defender.swarm
                && self.defender(candidate).entity != defender.entity
                && within(candidate)
            {
                let side = (candidate.position - defender.position).dot(tangent);
                positive += usize::from(side > 0.0);
                negative += usize::from(side < 0.0);
            }
            ControlFlow::<()>::Continue(())
        });
        (positive, negative)
    }
}

fn coarse_bucket(position: Vec2, size: f32) -> IVec2 {
    crate::spatial::floor_to_ivec2(position / size)
}

/// The coarse scan visits buckets row by row, then Defenders in insertion order.
fn coarse_order(a: &Candidate, b: &Candidate) -> Ordering {
    (a.coarse.y, a.coarse.x, a.order).cmp(&(b.coarse.y, b.coarse.x, b.order))
}

/// Decide `hostile > 0 && (friendly / hostile) as f32 < ratio` for sums taken in
/// another order, or `None` when rounding could change the answer.
///
/// A sum of `count` non-negative terms in any order lies within a relative
/// `count * EPSILON` of the exact sum, so the ratio from another order lies within
/// a relative `delta` of `friendly / hostile`. The `f32` conversion is monotonic,
/// so the answer is settled when both ends of that interval agree.
fn bounded_outnumbered(friendly: f64, hostile: f64, count: u64, ratio: f32) -> Option<bool> {
    if !(friendly.is_finite() && hostile.is_finite()) {
        return None;
    }
    // A sum of non-negative terms is positive exactly when one term is, in any order.
    if hostile <= 0.0 {
        return Some(false);
    }
    let estimate = friendly / hostile;
    let delta = 8.0 * (count as f64 + 2.0) * f64::EPSILON;
    if hostile < f64::MIN_POSITIVE / f64::EPSILON || !estimate.is_finite() || delta >= 0.5 {
        return None;
    }
    if ((estimate * (1.0 + delta)) as f32) < ratio {
        Some(true)
    } else if ((estimate * (1.0 - delta)) as f32) >= ratio {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use rand::{RngExt, SeedableRng, rngs::StdRng};

    use super::*;
    use crate::spatial::FixedSpatialBuckets;

    /// The coarse bucket scans that the index must answer like.
    struct CoarseScan(FixedSpatialBuckets<EngagementDefender>, f32);

    impl CoarseScan {
        fn new(defenders: &[EngagementDefender], size: f32) -> Self {
            let mut buckets = FixedSpatialBuckets::new(size);
            for defender in defenders {
                buckets.insert(defender.position, *defender);
            }
            Self(buckets, size)
        }

        fn near(
            &self,
            position: Vec2,
            radius: f32,
            bucket_radius: i32,
        ) -> impl Iterator<Item = &EngagementDefender> {
            let radius_squared = radius * radius;
            self.0
                .neighbourhood(self.0.bucket_for_position(position), bucket_radius)
                .flat_map(|(_, entries)| entries)
                .filter(move |defender| {
                    position.distance_squared(defender.position) <= radius_squared
                })
        }

        fn nearest(
            &self,
            position: Vec2,
            swarm: SwarmId,
            radius: f32,
            bucket_radius: i32,
        ) -> Option<(f32, Entity)> {
            self.near(position, radius, bucket_radius)
                .filter(|other| other.swarm != swarm)
                .map(|other| (position.distance_squared(other.position), other))
                .min_by(|(a_distance, a), (b_distance, b)| {
                    a_distance
                        .total_cmp(b_distance)
                        .then_with(|| a.key.cmp(&b.key))
                })
                .map(|(distance, other)| (distance, other.entity))
        }

        fn outnumbered(&self, position: Vec2, swarm: SwarmId, radius: f32, ratio: f32) -> bool {
            let bucket_radius = (radius / self.1).ceil() as i32;
            let (mut friendly, mut hostile) = (0.0, 0.0);
            for nearby in self.near(position, radius, bucket_radius) {
                if nearby.swarm != swarm {
                    hostile += nearby.strength;
                } else if !nearby.routing {
                    friendly += nearby.strength;
                }
            }
            hostile > 0.0 && ((friendly / hostile) as f32) < ratio
        }

        fn friendly_sides(
            &self,
            defender: &EngagementDefender,
            radius: f32,
            tangent: Vec2,
        ) -> (usize, usize) {
            let (mut positive, mut negative) = (0, 0);
            for (_, neighbours) in self
                .0
                .neighbourhood(self.0.bucket_for_position(defender.position), 1)
            {
                for other in neighbours {
                    let offset = other.position - defender.position;
                    if other.entity != defender.entity
                        && other.swarm == defender.swarm
                        && offset.length_squared() <= radius.powi(2)
                    {
                        let side = offset.dot(tangent);
                        positive += usize::from(side > 0.0);
                        negative += usize::from(side < 0.0);
                    }
                }
            }
            (positive, negative)
        }
    }

    fn defender(
        order: usize,
        key: u64,
        position: Vec2,
        swarm: u32,
        strength: f64,
    ) -> EngagementDefender {
        EngagementDefender {
            entity: Entity::from_raw_u32(order as u32 + 1).unwrap(),
            key,
            position,
            swarm: SwarmId(swarm),
            strength,
            routing: false,
            using_charger: false,
            evacuating: false,
        }
    }

    fn random_defenders(rng: &mut StdRng, count: usize, spread: f32) -> Vec<EngagementDefender> {
        let strengths = [0.0, 0.2, 0.4, 0.5, 1.0];
        (0..count)
            .map(|order| {
                // Snapped positions and shared keys produce exact distance and key ties.
                let position = if rng.random_range(0..4) == 0 {
                    Vec2::new(
                        (rng.random_range(-spread..spread) / 32.0).round() * 32.0,
                        (rng.random_range(-spread..spread) / 32.0).round() * 32.0,
                    )
                } else {
                    Vec2::new(
                        rng.random_range(-spread..spread),
                        rng.random_range(-spread..spread),
                    )
                };
                let strength = if rng.random_range(0..2) == 0 {
                    strengths[rng.random_range(0..strengths.len())]
                } else {
                    rng.random_range(0.0..1.0)
                };
                let mut defender = defender(
                    order,
                    rng.random_range(0..count as u64 / 2 + 1),
                    position,
                    rng.random_range(0..3),
                    strength,
                );
                defender.routing = rng.random_range(0..5) == 0;
                defender
            })
            .collect()
    }

    #[test]
    fn queries_match_a_coarse_bucket_scan() {
        let mut rng = StdRng::seed_from_u64(0x0089_1DE5);
        for layout in 0..80 {
            // The widest spread outgrows the fine-cell budget, so cells widen.
            let spread = [200.0, 900.0, 3_000.0, 40_000.0][layout % 4];
            let size = [240.0, 136.0, 1_000.0][layout / 4 % 3];
            let count = rng.random_range(1..400);
            let mut defenders = random_defenders(&mut rng, count, spread);
            // Some layouts hold Defenders no cell can place, or strengths the
            // fixed-point totals cannot hold, which disable the faster tiers.
            match layout / 12 % 4 {
                1 => defenders[0].position = Vec2::NAN,
                2 => defenders[0].strength = 300.0,
                3 => defenders[0].strength = -0.5,
                _ => {}
            }
            let scan = CoarseScan::new(&defenders, size);
            let index = DefenderIndex::new(defenders.clone(), size);
            for _ in 0..200 {
                let probe = if rng.random_range(0..2) == 0 {
                    defenders[rng.random_range(0..defenders.len())].position
                } else {
                    Vec2::new(
                        rng.random_range(-spread * 1.5..spread * 1.5),
                        rng.random_range(-spread * 1.5..spread * 1.5),
                    )
                };
                let swarm = SwarmId(rng.random_range(0..4));
                let radius =
                    [0.0, 32.0, 136.0, 240.0, 384.0, 768.0, 5_000.0][rng.random_range(0..7)];
                let ratio = [0.0, 0.4, 0.5, 1.0, 1.6][rng.random_range(0..5)];
                let bucket_radius = (radius / size).ceil() as i32;
                assert_eq!(
                    index.any_hostile(probe, swarm, radius),
                    scan.near(probe, radius, bucket_radius)
                        .any(|other| other.swarm != swarm),
                    "any hostile, layout {layout}, probe {probe:?}, radius {radius}",
                );
                for coarse_radius in [None, Some(1)] {
                    assert_eq!(
                        index
                            .nearest_hostile(probe, swarm, radius, coarse_radius)
                            .map(|(distance, other)| (distance, other.entity)),
                        scan.nearest(probe, swarm, radius, coarse_radius.unwrap_or(bucket_radius)),
                        "nearest hostile, layout {layout}, probe {probe:?}, radius {radius}",
                    );
                }
                assert_eq!(
                    index.outnumbered(probe, swarm, radius, ratio),
                    scan.outnumbered(probe, swarm, radius, ratio),
                    "outnumbered, layout {layout}, probe {probe:?}, radius {radius}, ratio {ratio}",
                );
                let member = defenders[rng.random_range(0..defenders.len())];
                let tangent = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU));
                assert_eq!(
                    index.friendly_sides(&member, radius, tangent),
                    scan.friendly_sides(&member, radius, tangent),
                    "friendly sides, layout {layout}, radius {radius}",
                );
            }
        }
    }

    #[test]
    fn a_ratio_on_an_f32_rounding_boundary_is_summed_in_coarse_order() {
        let below = 0.4_f32;
        let above = f32::from_bits(below.to_bits() + 1);
        let midpoint = (f64::from(below) + f64::from(above)) / 2.0;
        assert_eq!(bounded_outnumbered(midpoint, 1.0, 2, above), None);

        // Three friendly strengths whose sum crosses the midpoint in one order only.
        let (first, second, third) = (
            0.0052671968966553654,
            0.3947328239649702,
            2.7755575615628914e-17,
        );
        let coarse_order_sum = (first + second) + third;
        let fine_order_sum = (second + third) + first;
        assert_ne!(
            (coarse_order_sum as f32) < above,
            (fine_order_sum as f32) < above,
            "the orders must disagree for this check to mean anything",
        );
        // Coarse buckets visit `first` and `second` (bucket -1, 0) before `third`
        // (bucket 0, 0); fine rows visit `second` and `third` before `first`.
        let defenders = vec![
            defender(0, 0, Vec2::new(10.0, 5.0), 1, 1.0),
            defender(1, 1, Vec2::new(-100.0, 200.0), 0, first),
            defender(2, 2, Vec2::new(-150.0, 10.0), 0, second),
            defender(3, 3, Vec2::new(100.0, 20.0), 0, third),
        ];
        let scan = CoarseScan::new(&defenders, 240.0);
        let index = DefenderIndex::new(defenders, 240.0);
        for ratio in [below, above] {
            assert_eq!(
                index.outnumbered(Vec2::ZERO, SwarmId(0), 384.0, ratio),
                scan.outnumbered(Vec2::ZERO, SwarmId(0), 384.0, ratio),
                "ratio {ratio}",
            );
        }
    }
}
