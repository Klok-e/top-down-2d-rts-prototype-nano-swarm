use bevy::{math::DVec2, prelude::*};

use super::{BODY_RADIUS, CELL_WIDTH, Navigation, Obstacle, ObstacleIndex, expanded_rectangle};
use crate::nanobot::InteractionRegion;

#[derive(Clone, Copy)]
struct ContactBand {
    center: Vec2,
    radii: [f32; 2],
}

impl Navigation {
    pub(crate) fn tile_ground_point(
        &self,
        home: Vec2,
        min: Vec2,
        max: Vec2,
        tile_center: Vec2,
    ) -> Option<Vec2> {
        let corners = [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)];
        if self
            .obstacle_index
            .candidates(min, max)
            .into_iter()
            .any(|id| {
                corners
                    .iter()
                    .all(|&point| !self.obstacles[id].admits_body(point))
            })
        {
            return None;
        }

        let first = Self::cell(min);
        let last = Self::cell(max);
        let mut centers: Vec<_> = cells_between(first, last)
            .map(Self::center)
            .filter(|point| point.cmpge(min).all() && point.cmple(max).all())
            .collect();
        centers.sort_by(|a, b| {
            a.distance_squared(tile_center)
                .total_cmp(&b.distance_squared(tile_center))
        });
        if let Some(point) = self.connected_point(home, centers) {
            return Some(point);
        }
        if let Some(point) = self.visible_region_point(home, min, max, None, |_| true) {
            return Some(point);
        }
        self.connected_point(
            home,
            self.connector_probes(min, max, None, |_| true).flatten(),
        )
    }

    /// Yield one visibility sweep per origin so runtime navigation can budget each probe.
    pub(crate) fn visible_interaction_probes(
        &self,
        start: Vec2,
        region: InteractionRegion,
    ) -> impl Iterator<Item = Option<Vec2>> + '_ {
        let bounds = region
            .deposit_tile_band()
            .and_then(|(min, max, center, radii)| {
                let outer = Vec2::splat(radii[1]);
                let min = min.max(center - outer).max(self.min);
                let max = max.min(center + outer).min(self.max);
                (min.cmple(max).all() && self.point_clear(start)).then_some((
                    min,
                    max,
                    Some(ContactBand { center, radii }),
                ))
            });
        bounds.into_iter().flat_map(move |(min, max, band)| {
            std::iter::once_with(move || {
                self.visible_region_point(start, min, max, band, |point| region.contains(point))
            })
            .chain(self.connector_probes(min, max, band, move |point| region.contains(point)))
        })
    }

    fn connector_probes(
        &self,
        min: Vec2,
        max: Vec2,
        band: Option<ContactBand>,
        eligible: impl Fn(Vec2) -> bool,
    ) -> impl Iterator<Item = Option<Vec2>> {
        // Point routes can finish from a fine-cell center at most one cell from the goal.
        cells_between(Self::cell(min) - IVec2::ONE, Self::cell(max) + IVec2::ONE).map(move |cell| {
            let center = Self::center(cell);
            if (band.is_none() && center.cmpge(min).all() && center.cmple(max).all())
                || !self.point_clear(center)
            {
                return None;
            }
            let connector_min = (cell - IVec2::ONE).as_vec2() * CELL_WIDTH;
            let connector_max = (cell + IVec2::splat(2)).as_vec2() * CELL_WIDTH;
            self.visible_region_point(
                center,
                min.max(connector_min),
                max.min(connector_max),
                band,
                |point| (Self::cell(point) - cell).abs().cmple(IVec2::ONE).all() && eligible(point),
            )
        })
    }

    fn visible_region_point(
        &self,
        origin: Vec2,
        min: Vec2,
        max: Vec2,
        band: Option<ContactBand>,
        eligible: impl Fn(Vec2) -> bool,
    ) -> Option<Vec2> {
        if min.cmpgt(max).any() {
            return None;
        }
        if origin.cmpge(min).all() && origin.cmple(max).all() && eligible(origin) {
            return Some(origin);
        }

        let obstacles: Vec<_> = self
            .obstacle_index
            .swept_candidates(origin.min(min), origin.max(max))
            .into_iter()
            .map(|id| self.obstacles[id])
            .collect();
        let reachable = |point: Vec2| {
            point.cmpge(min).all()
                && point.cmple(max).all()
                && point.cmplt(self.max).all()
                && eligible(point)
                && obstacles.iter().all(|&obstacle| {
                    obstacle.admits_body(point) && obstacle.segment_clear(origin, point)
                })
        };

        // A clear segment first enters at a tile edge or on a work band's circular boundary.
        for (fixed, coordinate) in [(0, min.x), (0, max.x), (1, min.y), (1, max.y)] {
            if band.is_none()
                && (coordinate != origin[fixed].clamp(min[fixed], max[fixed])
                    || coordinate == origin[fixed])
            {
                continue;
            }
            let along = 1 - fixed;
            let mut edge = EdgeSweep::new(origin, fixed, coordinate, min[along], max[along]);
            for &obstacle in &obstacles {
                edge.add_obstacle(obstacle);
            }
            if let Some(band) = band {
                for radius in band.radii {
                    edge.split_at_disc_slice(band.center.as_dvec2(), f64::from(radius));
                }
            }
            if let Some(point) = edge.find_point(reachable) {
                return Some(point);
            }
        }
        let band = band?;
        band.radii.into_iter().rev().find_map(|radius| {
            let mut arc = ArcSweep::new(origin, band.center, radius);
            arc.split_at_box(min, max);
            for &obstacle in &obstacles {
                arc.add_obstacle(obstacle);
            }
            arc.find_point(reachable)
        })
    }
}

fn cells_between(min: IVec2, max: IVec2) -> impl Iterator<Item = IVec2> {
    (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| IVec2::new(x, y)))
}

impl ObstacleIndex {
    fn swept_candidates(&self, a: Vec2, b: Vec2) -> Vec<usize> {
        let (min, max) = Self::bounds(a, b);
        if Self::small_span(min, max) {
            return self.candidates(a, b);
        }
        // Wide queries visit occupied buckets inside the sweep bounds without walking empty space.
        let mut candidates = self.global.clone();
        for (&bucket, ids) in &self.buckets {
            if bucket.cmpge(min).all() && bucket.cmple(max).all() {
                candidates.extend_from_slice(ids);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

struct EdgeSweep {
    origin: DVec2,
    fixed: usize,
    coordinate: f32,
    low: f64,
    high: f64,
    cuts: Vec<f64>,
}

impl EdgeSweep {
    fn new(origin: Vec2, fixed: usize, coordinate: f32, low: f32, high: f32) -> Self {
        Self {
            origin: origin.as_dvec2(),
            fixed,
            coordinate,
            low: f64::from(low),
            high: f64::from(high),
            cuts: vec![f64::from(low), f64::from(high)],
        }
    }

    fn split_at(&mut self, value: f64) {
        if value > self.low && value < self.high {
            self.cuts.push(value);
        }
    }

    fn split_at_ray(&mut self, ray: DVec2, contact: f64) {
        let target = (f64::from(self.coordinate) - self.origin[self.fixed]) / ray[self.fixed];
        if target.is_finite() && target > 0.0 && target >= contact {
            let along = 1 - self.fixed;
            self.split_at(self.origin[along] + target * ray[along]);
        }
    }

    fn add_box(&mut self, min: Vec2, max: Vec2) {
        let along = 1 - self.fixed;
        if (min[self.fixed]..=max[self.fixed]).contains(&self.coordinate) {
            self.split_at(f64::from(min[along]));
            self.split_at(f64::from(max[along]));
        }
        for point in [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)] {
            self.split_at_ray(point.as_dvec2() - self.origin, 1.0);
        }
    }

    fn split_at_disc_slice(&mut self, center: DVec2, radius: f64) {
        let along = 1 - self.fixed;
        let separation = center[self.fixed] - f64::from(self.coordinate);
        let half_width_squared = radius * radius - separation * separation;
        if half_width_squared >= 0.0 {
            let half_width = half_width_squared.sqrt();
            self.split_at(center[along] - half_width);
            self.split_at(center[along] + half_width);
        }
    }

    fn add_disc(&mut self, center: Vec2, radius: f32) {
        let center = center.as_dvec2();
        let radius = f64::from(radius);
        self.split_at_disc_slice(center, radius);
        if let Some((rays, contact)) = disc_tangent_rays(center - self.origin, radius) {
            for ray in rays {
                self.split_at_ray(ray, contact);
            }
        }
    }

    fn add_obstacle(&mut self, obstacle: Obstacle) {
        match obstacle {
            Obstacle::Circle { center, radius } => self.add_disc(center, radius + BODY_RADIUS),
            Obstacle::Rectangle { center, half } => {
                let (strips, corners) = expanded_rectangle(center, half);
                for (min, max) in strips {
                    self.add_box(min, max);
                }
                for corner in corners {
                    self.add_disc(corner, BODY_RADIUS);
                }
            }
        }
    }

    fn find_point(mut self, reachable: impl Fn(Vec2) -> bool) -> Option<Vec2> {
        self.cuts.sort_unstable_by(f64::total_cmp);
        self.cuts.dedup();

        // Clearance can change when the destination reaches an obstacle or its ray becomes tangent.
        // Test the cuts as well as the spans to keep paths that only touch a formation.
        let spans = self.cuts.iter().copied().zip(
            self.cuts
                .iter()
                .copied()
                .skip(1)
                .chain(std::iter::once(self.high)),
        );
        for (low, high) in spans {
            let mut first = low as f32;
            let mut last = high as f32;
            if f64::from(first) < low {
                first = first.next_up();
            }
            if f64::from(last) > high {
                last = last.next_down();
            }
            if first > last {
                continue;
            }
            for along in [first.midpoint(last), first, last] {
                let mut candidate = Vec2::ZERO;
                candidate[self.fixed] = self.coordinate;
                candidate[1 - self.fixed] = along;
                if reachable(candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

struct ArcSweep {
    origin: DVec2,
    center: DVec2,
    radius: f64,
    cuts: Vec<f64>,
}

impl ArcSweep {
    fn new(origin: Vec2, center: Vec2, radius: f32) -> Self {
        Self {
            origin: origin.as_dvec2(),
            center: center.as_dvec2(),
            radius: f64::from(radius),
            cuts: vec![0.0, std::f64::consts::TAU],
        }
    }

    fn split_at(&mut self, radial: DVec2) {
        self.cuts
            .push(radial.y.atan2(radial.x).rem_euclid(std::f64::consts::TAU));
    }

    fn split_at_box(&mut self, min: Vec2, max: Vec2) {
        for (axis, coordinate) in [(0, min.x), (0, max.x), (1, min.y), (1, max.y)] {
            let depth = f64::from(coordinate) - self.center[axis];
            let cross_squared = self.radius * self.radius - depth * depth;
            if cross_squared >= 0.0 {
                let cross = cross_squared.sqrt();
                let mut radial = DVec2::ZERO;
                radial[axis] = depth;
                for along in [-cross, cross] {
                    radial[1 - axis] = along;
                    self.split_at(radial);
                }
            }
        }
    }

    fn split_at_disc(&mut self, center: DVec2, radius: f64) {
        let offset = center - self.center;
        let distance = offset.length();
        if distance == 0.0 {
            return;
        }
        let along =
            (self.radius * self.radius + distance * distance - radius * radius) / (2.0 * distance);
        let cross_squared = self.radius * self.radius - along * along;
        if cross_squared >= 0.0 {
            let axis = offset / distance;
            let cross = axis.perp() * cross_squared.sqrt();
            self.split_at(axis * along + cross);
            self.split_at(axis * along - cross);
        }
    }

    fn split_at_ray(&mut self, unit: DVec2, contact: f64) {
        let offset = self.center - self.origin;
        let depth = offset.dot(unit);
        let separation = offset.perp_dot(unit);
        let cross_squared = self.radius * self.radius - separation * separation;
        if cross_squared >= 0.0 {
            let cross = cross_squared.sqrt();
            for intersection in [depth - cross, depth + cross] {
                if intersection >= contact {
                    self.split_at(unit * intersection - offset);
                }
            }
        }
    }

    fn add_disc(&mut self, center: Vec2, radius: f32) {
        let center = center.as_dvec2();
        let radius = f64::from(radius);
        self.split_at_disc(center, radius);
        if let Some((rays, contact)) = disc_tangent_rays(center - self.origin, radius) {
            for ray in rays {
                self.split_at_ray(ray, contact);
            }
        }
    }

    fn add_obstacle(&mut self, obstacle: Obstacle) {
        match obstacle {
            Obstacle::Circle { center, radius } => self.add_disc(center, radius + BODY_RADIUS),
            Obstacle::Rectangle { center, half } => {
                let (strips, corners) = expanded_rectangle(center, half);
                for (min, max) in strips {
                    self.split_at_box(min, max);
                    for corner in [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)] {
                        let offset = corner.as_dvec2() - self.origin;
                        let distance = offset.length();
                        if distance > 0.0 {
                            self.split_at_ray(offset / distance, distance);
                        }
                    }
                }
                for corner in corners {
                    self.add_disc(corner, BODY_RADIUS);
                }
            }
        }
    }

    fn find_point(mut self, reachable: impl Fn(Vec2) -> bool) -> Option<Vec2> {
        self.cuts.sort_unstable_by(f64::total_cmp);
        self.cuts.dedup();
        for angles in self.cuts.windows(2) {
            for angle in [angles[0].midpoint(angles[1]), angles[0]] {
                let point = (self.center + DVec2::from_angle(angle) * self.radius).as_vec2();
                if reachable(point) {
                    return Some(point);
                }
            }
        }
        None
    }
}

fn disc_tangent_rays(offset: DVec2, radius: f64) -> Option<([DVec2; 2], f64)> {
    let length_squared = offset.length_squared();
    if length_squared == 0.0 {
        return None;
    }
    let contact = (length_squared - radius * radius).max(0.0).sqrt();
    let normal = offset.perp() * radius;
    let forward = offset * contact;
    Some((
        [
            (forward + normal) / length_squared,
            (forward - normal) / length_squared,
        ],
        contact,
    ))
}
