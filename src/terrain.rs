//! Permanent terrain footprints shared by movement and construction.

use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    platform::collections::HashMap,
    prelude::*,
};

use crate::navigation::{BODY_RADIUS, Obstacle};

/// Impassable, unowned terrain. Dimensions are world units; translation locates the shape.
#[derive(Component, Debug, Clone, Copy)]
#[component(on_add = index_rock, on_remove = unindex_rock)]
pub enum RockFormation {
    Rectangle { half: Vec2 },
    Circle { radius: f32 },
}

impl RockFormation {
    pub fn obstacle(self, transform: &Transform) -> Obstacle {
        let center = transform.translation.truncate();
        match self {
            Self::Rectangle { half } => Obstacle::Rectangle { center, half },
            Self::Circle { radius } => Obstacle::Circle { center, radius },
        }
    }
}

/// Spatial index over every rock of the current session. Rocks enter and leave it with
/// their `RockFormation` component, so a restart or scenario switch drops stale terrain.
#[derive(Resource, Default, Clone)]
pub struct TerrainIndex(Arc<RockIndex>);

impl TerrainIndex {
    pub(crate) fn rocks(&self) -> Arc<RockIndex> {
        self.0.clone()
    }
}

fn index_rock(mut world: DeferredWorld, context: HookContext) {
    let Some(transform) = world.get::<Transform>(context.entity).copied() else {
        return;
    };
    let Some(rock) = world.get::<RockFormation>(context.entity).copied() else {
        return;
    };
    let obstacle = rock.obstacle(&transform);
    let entity = context.entity;
    if let Some(mut index) = world.get_resource_mut::<TerrainIndex>() {
        Arc::make_mut(&mut index.0).insert(entity, obstacle);
    } else {
        world.commands().queue(move |world: &mut World| {
            let mut index = world.get_resource_or_insert_with(TerrainIndex::default);
            Arc::make_mut(&mut index.0).insert(entity, obstacle);
        });
    }
}

fn unindex_rock(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut index) = world.get_resource_mut::<TerrainIndex>() {
        Arc::make_mut(&mut index.0).remove(context.entity);
    }
}

/// Bucketed rock rectangles and circles with answers identical to scanning all of them.
#[derive(Default, Clone)]
pub(crate) struct RockIndex {
    rocks: BTreeMap<Entity, Obstacle>,
    buckets: HashMap<IVec2, Vec<IndexedRock>>,
    global: Vec<IndexedRock>,
}

/// A rock with the box beyond which it cannot constrain a body.
#[derive(Clone, Copy)]
struct IndexedRock {
    entity: Entity,
    shape: Obstacle,
    min: Vec2,
    max: Vec2,
}

impl IndexedRock {
    fn new(entity: Entity, shape: Obstacle) -> Self {
        let (min, max) = RockIndex::bounds(shape);
        Self {
            entity,
            shape,
            min,
            max,
        }
    }

    /// A query box wholly outside the reach box cannot touch this rock, so its answer
    /// needs no exact test. Non-finite boxes are never treated as separated.
    fn separated_from(&self, low: Vec2, high: Vec2) -> bool {
        low.cmpgt(self.max).any() || high.cmplt(self.min).any()
    }
}

impl RockIndex {
    const BUCKET: f32 = 256.0;
    const MAX_BUCKETS: i64 = 64;

    fn bounds(shape: Obstacle) -> (Vec2, Vec2) {
        let (center, half) = match shape {
            Obstacle::Rectangle { center, half } => (center, half),
            Obstacle::Circle { center, radius } => (center, Vec2::splat(radius)),
        };
        // Reach of body clearance plus slack for rounding.
        let half = half + Vec2::splat(BODY_RADIUS + 1.0);
        (center - half, center + half)
    }

    fn span(min: Vec2, max: Vec2) -> (IVec2, IVec2) {
        (
            (min / Self::BUCKET).floor().as_ivec2(),
            (max / Self::BUCKET).floor().as_ivec2(),
        )
    }

    fn small(min: IVec2, max: IVec2) -> bool {
        (i64::from(max.x) - i64::from(min.x) + 1) * (i64::from(max.y) - i64::from(min.y) + 1)
            <= Self::MAX_BUCKETS
    }

    fn insert(&mut self, entity: Entity, shape: Obstacle) {
        self.remove(entity);
        self.rocks.insert(entity, shape);
        let rock = IndexedRock::new(entity, shape);
        let (min, max) = Self::span(rock.min, rock.max);
        if !Self::small(min, max) {
            self.global.push(rock);
            return;
        }
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                self.buckets.entry(IVec2::new(x, y)).or_default().push(rock);
            }
        }
    }

    fn remove(&mut self, entity: Entity) {
        let Some(shape) = self.rocks.remove(&entity) else {
            return;
        };
        let (low, high) = Self::bounds(shape);
        let (min, max) = Self::span(low, high);
        if !Self::small(min, max) {
            self.global.retain(|rock| rock.entity != entity);
            return;
        }
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let key = IVec2::new(x, y);
                if let Some(bucket) = self.buckets.get_mut(&key) {
                    bucket.retain(|rock| rock.entity != entity);
                    if bucket.is_empty() {
                        self.buckets.remove(&key);
                    }
                }
            }
        }
    }

    /// Every rock in stable entity order.
    pub(crate) fn all(&self) -> impl Iterator<Item = Obstacle> + '_ {
        self.rocks.values().copied()
    }

    /// Whether every rock that could constrain the query box satisfies `accepts`.
    fn all_near(&self, a: Vec2, b: Vec2, accepts: impl Fn(Obstacle) -> bool) -> bool {
        if self.rocks.is_empty() {
            return true;
        }
        let (low, high) = (a.min(b), a.max(b));
        let (min, max) = Self::span(low, high);
        if !Self::small(min, max) {
            return self.rocks.values().all(|shape| accepts(*shape));
        }
        let near = |rock: &IndexedRock| rock.separated_from(low, high) || accepts(rock.shape);
        self.global.iter().all(near)
            && (min.y..=max.y).all(|y| {
                (min.x..=max.x).all(|x| {
                    self.buckets
                        .get(&IVec2::new(x, y))
                        .is_none_or(|bucket| bucket.iter().all(near))
                })
            })
    }

    pub(crate) fn admits_body(&self, point: Vec2) -> bool {
        self.all_near(point, point, |shape| shape.admits_body(point))
    }

    pub(crate) fn segment_clear(&self, start: Vec2, end: Vec2) -> bool {
        self.all_near(start, end, |shape| shape.segment_clear(start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(shapes: &[Obstacle]) -> RockIndex {
        let mut world = World::new();
        let mut index = RockIndex::default();
        for &shape in shapes {
            index.insert(world.spawn_empty().id(), shape);
        }
        index
    }

    /// Probes at, just inside, and just beyond body clearance from each rock's surface.
    fn clearance_probes(shape: Obstacle) -> Vec<Vec2> {
        let offsets = [-1.0, -0.01, 0.0, 0.01, 0.5, 1.0, 1.01, 2.0];
        let directions =
            (0..16).map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 16.0));
        let mut probes = Vec::new();
        for direction in directions {
            for offset in offsets {
                let reach = BODY_RADIUS + offset;
                probes.push(match shape {
                    Obstacle::Circle { center, radius } => center + direction * (radius + reach),
                    Obstacle::Rectangle { center, half } => {
                        // Edge midpoints for axis directions, corners otherwise.
                        let corner = center + half * direction.signum();
                        if direction.x.abs() < 1e-3 || direction.y.abs() < 1e-3 {
                            center + (half + Vec2::splat(reach)) * direction.round()
                        } else {
                            corner + direction * reach
                        }
                    }
                });
            }
        }
        probes
    }

    #[test]
    fn answers_at_the_edge_of_body_clearance_match_every_rock() {
        let shapes = [
            Obstacle::Rectangle {
                center: Vec2::new(300.0, 40.0),
                half: Vec2::new(100.0, 60.0),
            },
            Obstacle::Circle {
                center: Vec2::new(-500.0, -200.0),
                radius: 80.0,
            },
        ];
        let rocks = index(&shapes);
        let mut blocked = 0;
        for &shape in &shapes {
            for probe in clearance_probes(shape) {
                let expected = shapes.iter().all(|rock| rock.admits_body(probe));
                blocked += usize::from(!expected);
                assert_eq!(rocks.admits_body(probe), expected, "point {probe}");
                for delta in [Vec2::X, Vec2::Y, Vec2::ONE].map(|step| step * 150.0) {
                    for (start, end) in [(probe - delta, probe + delta), (probe, probe + delta)] {
                        let expected = shapes.iter().all(|rock| rock.segment_clear(start, end));
                        assert_eq!(
                            rocks.segment_clear(start, end),
                            expected,
                            "segment {start} -> {end}"
                        );
                    }
                }
            }
        }
        assert!(blocked > 0, "probes must reach inside body clearance");
    }

    #[test]
    fn an_empty_index_admits_everything() {
        let rocks = RockIndex::default();
        assert!(rocks.admits_body(Vec2::ZERO));
        assert!(rocks.segment_clear(Vec2::splat(-1.0e5), Vec2::splat(1.0e5)));
    }
}
