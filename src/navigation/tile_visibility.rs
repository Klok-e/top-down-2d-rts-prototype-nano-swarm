use bevy::{math::DVec2, prelude::*};
use std::future::ready;

use super::{
    BODY_RADIUS, CELL_WIDTH, Navigation, Obstacle, ObstacleIndex,
    budget::{WorkUnit, poll_with_budget},
    expanded_rectangle,
};
use crate::nanobot::InteractionRegion;

#[derive(Clone, Copy)]
struct ContactBand {
    center: Vec2,
    radii: [f32; 2],
    region: InteractionRegion,
}

#[derive(Clone, Copy)]
pub(super) struct ProbeBounds {
    min: Vec2,
    max: Vec2,
    band: Option<ContactBand>,
    connector: Option<IVec2>,
}

impl ProbeBounds {
    fn tile(min: Vec2, max: Vec2) -> Self {
        Self {
            min,
            max,
            band: None,
            connector: None,
        }
    }

    fn admits(self, point: Vec2) -> bool {
        point.cmpge(self.min).all()
            && point.cmple(self.max).all()
            && self.band.is_none_or(|band| band.region.contains(point))
            && self.connector.is_none_or(|cell| {
                (Navigation::cell(point) - cell)
                    .abs()
                    .cmple(IVec2::ONE)
                    .all()
            })
    }
}

struct Visibility<'a> {
    navigation: &'a Navigation,
    origin: Vec2,
    bounds: ProbeBounds,
    obstacles: Vec<usize>,
}

impl Visibility<'_> {
    async fn reachable(&self, point: Vec2) -> bool {
        WorkUnit::new().await;
        if !self.bounds.admits(point) || !point.cmplt(self.navigation.max).all() {
            return false;
        }
        for ids in self.obstacles.chunks(8) {
            WorkUnit::new().await;
            if !ids.iter().all(|&id| {
                let obstacle = self.navigation.obstacles[id];
                obstacle.admits_body(point) && obstacle.segment_clear(self.origin, point)
            }) {
                return false;
            }
        }
        true
    }
}

pub(super) enum InteractionProbe {
    Sample(Vec2),
    Direct(ProbeBounds),
    Connector(IVec2, ProbeBounds),
}

impl Navigation {
    pub(crate) fn tile_ground_point(
        &self,
        home: Vec2,
        min: Vec2,
        max: Vec2,
        tile_center: Vec2,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec2>, ()> {
        poll_with_budget(
            self.find_tile_ground(home, min, max, tile_center),
            allow_work,
        )
    }

    async fn find_tile_ground(
        &self,
        home: Vec2,
        min: Vec2,
        max: Vec2,
        tile_center: Vec2,
    ) -> Option<Vec2> {
        let corners = [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)];
        for id in self.obstacle_index.swept_candidates(min, max).await {
            WorkUnit::new().await;
            if corners
                .iter()
                .all(|&point| !self.obstacles[id].admits_body(point))
            {
                return None;
            }
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
        if let Some(point) = self
            .connected_point(home, centers.into_iter().map(|point| ready(Some(point))))
            .await
        {
            return Some(point);
        }
        let bounds = ProbeBounds::tile(min, max);
        if let Some(point) = self.visible_region_point(home, bounds).await {
            return Some(point);
        }
        let probes = connector_cells(min, max).map(|cell| self.connector_probe(cell, bounds));
        self.connected_point(home, probes).await
    }

    pub(crate) fn visible_tile_point(
        &self,
        origin: Vec2,
        min: Vec2,
        max: Vec2,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec2>, ()> {
        let center = min.midpoint(max);
        if self.segment_clear_with_budget(origin, center, allow_work)? {
            return Ok(Some(center));
        }
        poll_with_budget(
            self.visible_region_point(origin, ProbeBounds::tile(min, max)),
            allow_work,
        )
    }

    pub(crate) fn connected_interaction_with_budget(
        &self,
        start: Vec2,
        region: InteractionRegion,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec2>, ()> {
        poll_with_budget(
            async {
                let probes = self
                    .interaction_probes(start, region, true, true)
                    .await
                    .map(|probe| self.resolve_interaction_probe(start, probe));
                self.connected_point(start, probes).await
            },
            allow_work,
        )
    }

    pub(super) async fn interaction_probes(
        &self,
        start: Vec2,
        region: InteractionRegion,
        start_clear: bool,
        meter_probes: bool,
    ) -> impl Iterator<Item = InteractionProbe> + use<> {
        if meter_probes {
            let (min, max) = region.candidate_cell_bounds();
            let width = (i64::from(max.x) - i64::from(min.x) + 1).max(0) as usize;
            let height = (i64::from(max.y) - i64::from(min.y) + 1).max(0) as usize;
            let cells = width.saturating_mul(height);
            // Each sampled cell can compare against all earlier cells and the analytic probes.
            let candidates = cells.saturating_add(30);
            charge_visibility_work(cells.saturating_mul(candidates)).await;
            charge_visibility_sort(candidates).await;
        }
        let bounds = region
            .deposit_tile_band()
            .and_then(|(min, max, center, radii)| {
                let outer = Vec2::splat(radii[1]);
                let min = min.max(center - outer).max(self.min);
                let max = max.min(center + outer).min(self.max);
                (start_clear && min.cmple(max).all()).then_some(ProbeBounds {
                    min,
                    max,
                    band: Some(ContactBand {
                        center,
                        radii,
                        region,
                    }),
                    connector: None,
                })
            });
        let exact_probes = bounds.into_iter().flat_map(|bounds| {
            std::iter::once(InteractionProbe::Direct(bounds)).chain(
                connector_cells(bounds.min, bounds.max)
                    .map(move |cell| InteractionProbe::Connector(cell, bounds)),
            )
        });
        region
            .candidates(start)
            .into_iter()
            .map(InteractionProbe::Sample)
            .chain(exact_probes)
    }

    pub(super) async fn resolve_interaction_probe(
        &self,
        start: Vec2,
        probe: InteractionProbe,
    ) -> Option<Vec2> {
        match probe {
            InteractionProbe::Sample(point) => Some(point),
            InteractionProbe::Direct(bounds) => self.visible_region_point(start, bounds).await,
            InteractionProbe::Connector(cell, bounds) => self.connector_probe(cell, bounds).await,
        }
    }

    async fn connector_probe(&self, cell: IVec2, bounds: ProbeBounds) -> Option<Vec2> {
        let center = Self::center(cell);
        if (bounds.band.is_none() && bounds.admits(center)) || !self.async_point_clear(center).await
        {
            return None;
        }
        let connector_min = (cell - IVec2::ONE).as_vec2() * CELL_WIDTH;
        let connector_max = (cell + IVec2::splat(2)).as_vec2() * CELL_WIDTH;
        self.visible_region_point(
            center,
            ProbeBounds {
                min: bounds.min.max(connector_min),
                max: bounds.max.min(connector_max),
                connector: Some(cell),
                ..bounds
            },
        )
        .await
    }

    async fn visible_region_point(&self, origin: Vec2, bounds: ProbeBounds) -> Option<Vec2> {
        let ProbeBounds { min, max, band, .. } = bounds;
        if min.cmpgt(max).any() {
            return None;
        }
        if bounds.admits(origin) {
            return Some(origin);
        }

        let obstacles = self
            .obstacle_index
            .swept_candidates(origin.min(min), origin.max(max))
            .await;
        let sweep = Visibility {
            navigation: self,
            origin,
            bounds,
            obstacles,
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
            for &id in &sweep.obstacles {
                WorkUnit::new().await;
                edge.add_obstacle(self.obstacles[id]);
            }
            if let Some(band) = band {
                for radius in band.radii {
                    edge.split_at_disc_slice(band.center.as_dvec2(), f64::from(radius));
                }
            }
            if let Some(point) = edge.find_point(&sweep).await {
                return Some(point);
            }
        }
        let band = band?;
        for radius in band.radii.into_iter().rev() {
            let mut arc = ArcSweep::new(origin, band.center, radius);
            arc.split_at_box(min, max);
            for &id in &sweep.obstacles {
                WorkUnit::new().await;
                arc.add_obstacle(self.obstacles[id]);
            }
            if let Some(point) = arc.find_point(&sweep).await {
                return Some(point);
            }
        }
        None
    }
}

// Point routes can finish from a fine-cell center at most one cell from the goal.
fn connector_cells(min: Vec2, max: Vec2) -> impl Iterator<Item = IVec2> {
    cells_between(
        Navigation::cell(min) - IVec2::ONE,
        Navigation::cell(max) + IVec2::ONE,
    )
}

fn cells_between(min: IVec2, max: IVec2) -> impl Iterator<Item = IVec2> {
    (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| IVec2::new(x, y)))
}

impl ObstacleIndex {
    pub(super) async fn clearance_candidates(
        &self,
        a: Vec2,
        b: Vec2,
    ) -> impl Iterator<Item = usize> + '_ {
        let (min, max) = Self::bounds(a, b);
        let (bucket, candidates, all) = if min == max && self.global.is_empty() {
            let bucket = self.buckets.get(&min).map_or(&[][..], Vec::as_slice);
            (bucket, Vec::new(), 0..0)
        } else if Self::small_span(min, max) {
            (&[][..], self.collect_sweep_candidates(min, max).await, 0..0)
        } else {
            // A wide segment checks all obstacles in their original order.
            (&[][..], Vec::new(), 0..self.count)
        };
        bucket.iter().copied().chain(candidates).chain(all)
    }

    async fn swept_candidates(&self, a: Vec2, b: Vec2) -> Vec<usize> {
        let (min, max) = Self::bounds(a, b);
        self.collect_sweep_candidates(min, max).await
    }

    async fn collect_sweep_candidates(&self, min: IVec2, max: IVec2) -> Vec<usize> {
        let mut buckets = Vec::new();
        if !self.global.is_empty() {
            buckets.push(self.global.as_slice());
        }
        if Self::small_span(min, max) {
            for y in min.y..=max.y {
                for x in min.x..=max.x {
                    if let Some(ids) = self.buckets.get(&IVec2::new(x, y)) {
                        buckets.push(ids);
                    }
                }
            }
        } else {
            // Wide queries scan the occupied-bucket index instead of walking empty space.
            charge_visibility_work(self.buckets.capacity()).await;
            for (&bucket, ids) in &self.buckets {
                if bucket.cmpge(min).all() && bucket.cmple(max).all() {
                    buckets.push(ids);
                }
            }
        }
        charge_visibility_sort(buckets.iter().map(|ids| ids.len()).sum()).await;
        let mut candidates: Vec<_> = buckets.into_iter().flatten().copied().collect();
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

async fn charge_visibility_work(items: usize) {
    // Bound repeated bookkeeping in batches so sparse sweeps retain their low overhead.
    for _ in 0..items / 64 {
        WorkUnit::new().await;
    }
}

async fn charge_visibility_sort(items: usize) {
    let passes = items.checked_ilog2().unwrap_or(0) as usize + 1;
    charge_visibility_work(items.saturating_mul(passes)).await;
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

    async fn find_point(mut self, sweep: &Visibility<'_>) -> Option<Vec2> {
        charge_visibility_sort(self.cuts.len()).await;
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
                WorkUnit::new().await;
                continue;
            }
            for along in [first.midpoint(last), first, last] {
                let mut candidate = Vec2::ZERO;
                candidate[self.fixed] = self.coordinate;
                candidate[1 - self.fixed] = along;
                if sweep.reachable(candidate).await {
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

    async fn find_point(mut self, sweep: &Visibility<'_>) -> Option<Vec2> {
        charge_visibility_sort(self.cuts.len()).await;
        self.cuts.sort_unstable_by(f64::total_cmp);
        self.cuts.dedup();
        for angles in self.cuts.windows(2) {
            for angle in [angles[0].midpoint(angles[1]), angles[0]] {
                let point = (self.center + DVec2::from_angle(angle) * self.radius).as_vec2();
                if sweep.reachable(point).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::IntentGrid;

    #[test]
    fn blocked_tile_in_dense_geometry_requires_more_sweep_allowance() {
        let mut obstacles = vec![Obstacle::Rectangle {
            center: Vec2::new(768.0, 300.0),
            half: Vec2::new(64.0, 512.0),
        }];
        obstacles.extend((0..512).map(|id| Obstacle::Circle {
            center: Vec2::new(3270.0 + id as f32 / 8.0, 1850.0),
            radius: 25.0,
        }));
        let navigation = Navigation::new(&IntentGrid::new(32, 32), obstacles);
        let home = Vec2::splat(36.0);
        let min = Vec2::new(4096.0, 1400.0);
        let max = Vec2::new(4607.0, 1911.0);
        assert_eq!(
            navigation.visible_tile_point(home, min, max, &mut || true),
            Ok(None),
        );

        let mut allowance = 16;
        assert_eq!(
            navigation.visible_tile_point(home, min, max, &mut || {
                let allowed = allowance > 0;
                allowance -= usize::from(allowed);
                allowed
            }),
            Err(()),
        );
    }
}
