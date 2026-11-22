use bevy::{
    platform::collections::HashMap,
    prelude::{IVec2, Vec2},
};

/// Deterministic fixed-size spatial buckets.
///
/// Consumers own separate instances and choose bucket sizes suited to their
/// queries. Values within each bucket retain insertion order until explicitly
/// sorted.
#[derive(Debug, Clone)]
pub struct FixedSpatialBuckets<T> {
    bucket_size: f32,
    buckets: HashMap<(i32, i32), Vec<T>>,
}

impl<T> FixedSpatialBuckets<T> {
    pub fn new(bucket_size: f32) -> Self {
        assert!(bucket_size.is_finite() && bucket_size > 0.0);
        Self {
            bucket_size,
            buckets: HashMap::default(),
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

/// Fixed-size spatial buckets built once from all their values.
///
/// Answers exactly like [`FixedSpatialBuckets`] filled in the same order, but stores
/// values contiguously, so a neighbourhood can also be read as a few runs of values.
#[derive(Debug, Clone)]
pub struct PackedSpatialBuckets<T> {
    bucket_size: f32,
    values: Vec<T>,
    layout: PackedLayout,
}

#[derive(Debug, Clone)]
enum PackedLayout {
    /// Row-major value ranges for every bucket between the occupied extremes.
    Dense {
        origin: (i64, i64),
        width: i64,
        height: i64,
        starts: Vec<u32>,
    },
    /// Value ranges of occupied buckets, for values spread too far for a dense grid.
    Sparse(HashMap<(i32, i32), (u32, u32)>),
}

impl<T> PackedSpatialBuckets<T> {
    /// Bucket `values` in iteration order, which each bucket keeps.
    pub fn new(bucket_size: f32, values: impl IntoIterator<Item = (Vec2, T)>) -> Self {
        assert!(bucket_size.is_finite() && bucket_size > 0.0);
        let probe = FixedSpatialBuckets::<()>::new(bucket_size);
        let placed: Vec<_> = values
            .into_iter()
            .map(|(position, value)| (probe.bucket_for_position(position), value))
            .collect();
        let (low, high) = placed.iter().fold(
            ((i64::MAX, i64::MAX), (i64::MIN, i64::MIN)),
            |(low, high), (coord, _)| {
                let (x, y) = (i64::from(coord.x), i64::from(coord.y));
                ((low.0.min(x), low.1.min(y)), (high.0.max(x), high.1.max(y)))
            },
        );
        let (width, height) = if placed.is_empty() {
            (0, 0)
        } else {
            (high.0 - low.0 + 1, high.1 - low.1 + 1)
        };
        let dense_limit = (4 * placed.len() as i64).max(4096);
        if width
            .checked_mul(height)
            .is_some_and(|cells| cells <= dense_limit)
        {
            let cell = |coord: IVec2| {
                ((i64::from(coord.y) - low.1) * width + i64::from(coord.x) - low.0) as usize
            };
            let mut starts = vec![0_u32; (width * height + 1) as usize];
            for (coord, _) in &placed {
                starts[cell(*coord) + 1] += 1;
            }
            for index in 1..starts.len() {
                starts[index] += starts[index - 1];
            }
            let mut slots: Vec<Option<T>> =
                std::iter::repeat_with(|| None).take(placed.len()).collect();
            let mut next = starts.clone();
            for (coord, value) in placed {
                let slot = &mut next[cell(coord)];
                slots[*slot as usize] = Some(value);
                *slot += 1;
            }
            Self {
                bucket_size,
                values: slots.into_iter().map(Option::unwrap).collect(),
                layout: PackedLayout::Dense {
                    origin: low,
                    width,
                    height,
                    starts,
                },
            }
        } else {
            let mut placed = placed;
            // A stable sort keeps each bucket's values in iteration order.
            placed.sort_by_key(|(coord, _)| (coord.y, coord.x));
            let mut ranges = HashMap::default();
            for (index, (coord, _)) in placed.iter().enumerate() {
                let range = ranges
                    .entry((coord.x, coord.y))
                    .or_insert((index as u32, index as u32));
                range.1 = index as u32 + 1;
            }
            Self {
                bucket_size,
                values: placed.into_iter().map(|(_, value)| value).collect(),
                layout: PackedLayout::Sparse(ranges),
            }
        }
    }

    pub fn bucket_for_position(&self, position: Vec2) -> IVec2 {
        IVec2::new(
            (position.x / self.bucket_size).floor() as i32,
            (position.y / self.bucket_size).floor() as i32,
        )
    }

    pub fn entries(&self, coord: IVec2) -> &[T] {
        match &self.layout {
            PackedLayout::Dense {
                origin,
                width,
                height,
                starts,
            } => {
                let (x, y) = (i64::from(coord.x) - origin.0, i64::from(coord.y) - origin.1);
                if !(0..*width).contains(&x) || !(0..*height).contains(&y) {
                    return &[];
                }
                let cell = (y * width + x) as usize;
                &self.values[starts[cell] as usize..starts[cell + 1] as usize]
            }
            PackedLayout::Sparse(ranges) => ranges
                .get(&(coord.x, coord.y))
                .map_or(&[], |&(start, end)| {
                    &self.values[start as usize..end as usize]
                }),
        }
    }

    pub fn neighbourhood(&self, center: IVec2, radius: i32) -> impl Iterator<Item = (IVec2, &[T])> {
        let radius = radius.max(0);
        (-radius..=radius).flat_map(move |dy| {
            (-radius..=radius).filter_map(move |dx| {
                let coord = center + IVec2::new(dx, dy);
                let entries = self.entries(coord);
                (!entries.is_empty()).then_some((coord, entries))
            })
        })
    }

    /// The values of [`Self::neighbourhood`], in its order, as contiguous runs.
    pub fn neighbourhood_runs(&self, center: IVec2, radius: i32) -> impl Iterator<Item = &[T]> {
        let radius = i64::from(radius.max(0));
        let rows = (-radius..=radius).flat_map(move |dy| {
            let runs: [Option<&[T]>; 1] = match &self.layout {
                PackedLayout::Dense {
                    origin,
                    width,
                    height,
                    starts,
                } => {
                    let y = i64::from(center.y) + dy - origin.1;
                    let first = (i64::from(center.x) - radius - origin.0).max(0);
                    let last = (i64::from(center.x) + radius - origin.0).min(width - 1);
                    [((0..*height).contains(&y) && first <= last).then(|| {
                        let row = y * width;
                        &self.values[starts[(row + first) as usize] as usize
                            ..starts[(row + last) as usize + 1] as usize]
                    })]
                }
                PackedLayout::Sparse(_) => [None],
            };
            runs.into_iter().flatten()
        });
        let sparse = matches!(self.layout, PackedLayout::Sparse(_))
            .then(|| {
                self.neighbourhood(center, radius as i32)
                    .map(|(_, entries)| entries)
            })
            .into_iter()
            .flatten();
        rows.chain(sparse).filter(|run| !run.is_empty())
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

    #[test]
    fn packed_buckets_answer_like_incremental_buckets() {
        use rand::{RngExt, SeedableRng, rngs::StdRng};

        let mut rng = StdRng::seed_from_u64(0x0089_BACE);
        // Clustered layouts pack densely; a few far outliers force the sparse layout.
        for (layout, spread, outliers) in [(0, 300.0, 0), (1, 3_000.0, 0), (2, 300.0, 3)] {
            let mut values: Vec<_> = (0..500)
                .map(|value| {
                    let position = Vec2::new(
                        rng.random_range(-spread..spread),
                        rng.random_range(-spread..spread),
                    );
                    (position, value)
                })
                .collect();
            for value in 0..outliers {
                values.push((Vec2::splat(1.0e7 * (value as f32 + 1.0)), 500 + value));
            }
            let mut incremental = FixedSpatialBuckets::new(73.0);
            for &(position, value) in &values {
                incremental.insert(position, value);
            }
            let packed = PackedSpatialBuckets::new(73.0, values.iter().copied());
            for _ in 0..300 {
                let center = incremental.bucket_for_position(Vec2::new(
                    rng.random_range(-spread * 1.2..spread * 1.2),
                    rng.random_range(-spread * 1.2..spread * 1.2),
                ));
                let radius = rng.random_range(-1..4);
                let expected: Vec<_> = incremental
                    .neighbourhood(center, radius)
                    .map(|(coord, entries)| (coord, entries.to_vec()))
                    .collect();
                let actual: Vec<_> = packed
                    .neighbourhood(center, radius)
                    .map(|(coord, entries)| (coord, entries.to_vec()))
                    .collect();
                assert_eq!(
                    actual, expected,
                    "layout {layout}, center {center}, radius {radius}"
                );
                let runs: Vec<_> = packed
                    .neighbourhood_runs(center, radius)
                    .flatten()
                    .copied()
                    .collect();
                let flattened: Vec<_> = expected
                    .into_iter()
                    .flat_map(|(_, entries)| entries)
                    .collect();
                assert_eq!(
                    runs, flattened,
                    "layout {layout}, center {center}, radius {radius}"
                );
            }
        }
    }
}
