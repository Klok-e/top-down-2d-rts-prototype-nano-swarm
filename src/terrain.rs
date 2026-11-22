//! Permanent terrain footprints shared by movement and construction.

use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    platform::collections::HashMap,
    prelude::*,
};

use crate::navigation::Obstacle;

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
        let (min, max) = shape.reach_box();
        Self {
            entity,
            shape,
            min,
            max,
        }
    }

    /// A query box wholly outside the reach box cannot touch this rock, so its answer
    /// needs no exact test.
    fn separated_from(&self, low: Vec2, high: Vec2) -> bool {
        crate::navigation::separated(low, high, (self.min, self.max))
    }
}

impl RockIndex {
    const BUCKET: f32 = 256.0;
    const MAX_BUCKETS: i64 = 64;

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
        let (low, high) = shape.reach_box();
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
