use std::collections::BTreeMap;

use bevy::prelude::{IVec2, Vec2};

/// Deterministic fixed-size spatial buckets.
///
/// Consumers own separate instances and choose bucket sizes suited to their
/// queries. Values within each bucket retain insertion order until explicitly
/// sorted.
#[derive(Debug, Clone)]
pub struct FixedSpatialBuckets<T> {
    bucket_size: f32,
    buckets: BTreeMap<(i32, i32), Vec<T>>,
}

impl<T> FixedSpatialBuckets<T> {
    pub fn new(bucket_size: f32) -> Self {
        assert!(bucket_size.is_finite() && bucket_size > 0.0);
        Self {
            bucket_size,
            buckets: BTreeMap::new(),
        }
    }

    pub fn bucket_for_position(&self, position: Vec2) -> IVec2 {
        IVec2::new(
            (position.x / self.bucket_size).floor() as i32,
            (position.y / self.bucket_size).floor() as i32,
        )
    }

    pub fn insert(&mut self, position: Vec2, value: T) {
        let coord = self.bucket_for_position(position);
        self.buckets
            .entry((coord.x, coord.y))
            .or_default()
            .push(value);
    }

    pub fn clear(&mut self) {
        self.buckets.clear();
    }

    pub fn entries(&self, coord: IVec2) -> &[T] {
        self.buckets
            .get(&(coord.x, coord.y))
            .map_or(&[], Vec::as_slice)
    }

    pub fn sort_entries_by<F>(&mut self, mut compare: F)
    where
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        for entries in self.buckets.values_mut() {
            entries.sort_by(&mut compare);
        }
    }

    pub fn neighbourhood(&self, center: IVec2, radius: i32) -> impl Iterator<Item = (IVec2, &[T])> {
        let radius = radius.max(0);
        (-radius..=radius).flat_map(move |dy| {
            (-radius..=radius).filter_map(move |dx| {
                let coord = center + IVec2::new(dx, dy);
                self.buckets
                    .get(&(coord.x, coord.y))
                    .map(|entries| (coord, entries.as_slice()))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_coordinates_floor_negative_world_positions() {
        let buckets = FixedSpatialBuckets::<u32>::new(32.0);

        assert_eq!(
            buckets.bucket_for_position(Vec2::new(0.0, 31.9)),
            IVec2::new(0, 0)
        );
        assert_eq!(
            buckets.bucket_for_position(Vec2::new(32.0, 0.0)),
            IVec2::new(1, 0)
        );
        assert_eq!(
            buckets.bucket_for_position(Vec2::new(-0.1, -32.0)),
            IVec2::new(-1, -1)
        );
        assert_eq!(
            buckets.bucket_for_position(Vec2::new(-32.1, 0.0)),
            IVec2::new(-2, 0)
        );
    }
}
