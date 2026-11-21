//! Scenario-stable ownership-aware intent bounds.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use bevy::prelude::{IVec2, Resource, Vec2};

use crate::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{SwarmId, world_to_cell},
    navigation::{Navigation, Obstacle},
};

/// The action performed by a gameplay intent writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentWriteAction {
    Paint,
    Erase,
}

/// A rejected gameplay-level intent write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IntentWriteError {
    #[error("intent cell is outside the map")]
    OutOfBounds,
    #[error("intent cell is outside the swarm-reachable map")]
    Unreachable,
}

/// A scenario cannot establish an intent boundary for this swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReachabilitySetupError {
    #[error("Swarm {0:?} has an unreachable home")]
    UnreachableHome(SwarmId),
    #[error("initial intent for Swarm {owner:?} at {cell} is outside the swarm-reachable map")]
    UnreachableInitialIntent { owner: SwarmId, cell: IVec2 },
}

/// Immutable permanent-terrain connectivity plus memoized owner/cell membership.
///
/// The resource deliberately owns a navigation instance separate from runtime navigation:
/// deposits, structures, and transient clearing barriers must not make the playable map
/// flicker. The navigation topology and each answer are retained for the scenario lifetime.
#[derive(Resource, Clone)]
pub struct SwarmReachabilityMap {
    inner: Arc<ReachabilityInner>,
}

impl std::fmt::Debug for SwarmReachabilityMap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SwarmReachabilityMap")
            .field("homes", &self.inner.homes)
            .finish_non_exhaustive()
    }
}

struct ReachabilityInner {
    navigation: Navigation,
    homes: BTreeMap<SwarmId, Vec2>,
    cells: Mutex<HashMap<(SwarmId, IVec2), Option<Vec2>>>,
    ground_witnesses: Mutex<HashMap<(SwarmId, IVec2), Vec2>>,
}

impl SwarmReachabilityMap {
    /// Build a scenario's permanent-terrain reachability boundary.
    pub fn new(
        grid: &IntentGrid,
        terrain: impl IntoIterator<Item = Obstacle>,
        homes: impl IntoIterator<Item = (SwarmId, Vec2)>,
    ) -> Result<Self, ReachabilitySetupError> {
        let navigation = Navigation::new(grid, terrain.into_iter().collect());
        let homes = homes.into_iter().collect::<BTreeMap<_, _>>();
        for (&owner, &home) in &homes {
            if !navigation.point_clear(home) {
                return Err(ReachabilitySetupError::UnreachableHome(owner));
            }
        }
        Ok(Self {
            inner: Arc::new(ReachabilityInner {
                navigation,
                homes,
                cells: Mutex::new(HashMap::new()),
                ground_witnesses: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub(crate) fn same_boundary(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Whether the cell contains ground accessible from the swarm's scenario home.
    pub fn contains(&self, owner: SwarmId, cell: IVec2, grid: &IntentGrid) -> bool {
        self.contains_with_budget(owner, cell, grid, &mut || true)
            .expect("unlimited cell reachability query")
    }

    pub(crate) fn contains_with_budget(
        &self,
        owner: SwarmId,
        cell: IVec2,
        grid: &IntentGrid,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<bool, ()> {
        if !grid.in_bounds(cell) {
            return Ok(false);
        }
        if self
            .inner
            .ground_witnesses
            .lock()
            .unwrap()
            .contains_key(&(owner, cell))
        {
            return Ok(true);
        }
        Ok(self
            .reachable_point_with_budget(owner, cell, grid, allow_work)?
            .is_some())
    }

    fn nearby_ground(&self, owner: SwarmId, cell: IVec2) -> [Option<Vec2>; 5] {
        let ground = self.inner.ground_witnesses.lock().unwrap();
        [IVec2::ZERO, IVec2::X, -IVec2::X, IVec2::Y, -IVec2::Y]
            .map(|offset| ground.get(&(owner, cell + offset)).copied())
    }

    /// Starting at verified ground, find and remember ground in a nearby tile.
    pub(crate) fn visible_ground_from(
        &self,
        owner: SwarmId,
        verified_origin: Vec2,
        cell: IVec2,
        grid: &IntentGrid,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec2>, ()> {
        if !grid.in_bounds(cell) {
            return Ok(None);
        }
        let (min, max) = tile_bounds(cell);
        let point =
            self.inner
                .navigation
                .visible_tile_point(verified_origin, min, max, allow_work)?;
        if let Some(point) = point {
            self.inner
                .ground_witnesses
                .lock()
                .unwrap()
                .entry((owner, cell))
                .or_insert(point);
        }
        Ok(point)
    }

    /// The returned route is checked against permanent terrain before its cells can be painted.
    pub(crate) fn remember_route_ground(
        &self,
        owner: SwarmId,
        start: Vec2,
        waypoints: &[Vec2],
        samples: &[Vec2],
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<(), ()> {
        let Some(&home) = self.inner.homes.get(&owner) else {
            return Ok(());
        };
        let navigation = &self.inner.navigation;
        let mut from = home;
        for to in std::iter::once(start).chain(waypoints.iter().copied()) {
            if !navigation.segment_clear_with_budget(from, to, allow_work)? {
                return Ok(());
            }
            from = to;
        }
        let mut ground = self.inner.ground_witnesses.lock().unwrap();
        for &point in samples {
            if !allow_work() {
                return Err(());
            }
            ground.entry((owner, world_to_cell(point))).or_insert(point);
        }
        Ok(())
    }

    /// A representative point in the cell, preferring its center when accessible.
    pub fn reachable_point(&self, owner: SwarmId, cell: IVec2, grid: &IntentGrid) -> Option<Vec2> {
        self.reachable_point_with_budget(owner, cell, grid, &mut || true)
            .expect("unlimited cell reachability query")
    }

    pub(crate) fn reachable_point_with_budget(
        &self,
        owner: SwarmId,
        cell: IVec2,
        grid: &IntentGrid,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec2>, ()> {
        if !grid.in_bounds(cell) {
            return Ok(None);
        }
        let Some(&home) = self.inner.homes.get(&owner) else {
            return Ok(None);
        };
        let cached = self
            .inner
            .cells
            .lock()
            .unwrap()
            .get(&(owner, cell))
            .copied();
        if let Some(cached) = cached {
            return Ok(cached.or_else(|| {
                self.inner
                    .ground_witnesses
                    .lock()
                    .unwrap()
                    .get(&(owner, cell))
                    .copied()
            }));
        }
        let min = cell.as_vec2() * ZONE_BLOCK_SIZE;
        let center = min + Vec2::splat(ZONE_BLOCK_SIZE / 2.0);
        let navigation = &self.inner.navigation;
        let witnesses = self.nearby_ground(owner, cell);
        for witness in witnesses.into_iter().flatten() {
            if navigation.segment_clear_with_budget(witness, center, allow_work)? {
                self.inner
                    .cells
                    .lock()
                    .unwrap()
                    .insert((owner, cell), Some(center));
                return Ok(Some(center));
            }
        }
        let mut point = navigation.connected_point_with_budget(home, [Some(center)], allow_work)?;
        if point.is_none() {
            if world_to_cell(home) == cell {
                point = Some(home);
            } else {
                let (_, max) = tile_bounds(cell);
                point = match witnesses[0] {
                    Some(witness) => Some(witness),
                    None => navigation.tile_ground_point(home, min, max, center, allow_work)?,
                };
            }
        }
        self.inner
            .cells
            .lock()
            .unwrap()
            .insert((owner, cell), point);
        Ok(point)
    }

    /// Whether a specific point is accessible with normal body clearance.
    pub fn position_is_reachable(&self, owner: SwarmId, position: Vec2, grid: &IntentGrid) -> bool {
        self.position_is_reachable_with_budget(owner, position, grid, &mut || true)
            .expect("unlimited position reachability query")
    }

    pub(crate) fn position_is_reachable_with_budget(
        &self,
        owner: SwarmId,
        position: Vec2,
        grid: &IntentGrid,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<bool, ()> {
        let navigation = &self.inner.navigation;
        let cell = world_to_cell(position);
        let Some(&home) = self.inner.homes.get(&owner) else {
            return Ok(false);
        };
        if !grid.in_bounds(cell) {
            return Ok(false);
        }
        if !allow_work() {
            return Err(());
        }
        if !navigation.point_clear(position) {
            return Ok(false);
        }
        let directly_reachable =
            navigation.segment_clear_with_budget(home, position, allow_work)?;
        let reachable = if directly_reachable {
            true
        } else {
            let ground = self.reachable_point_with_budget(owner, cell, grid, allow_work)?;
            let from_ground = match ground {
                Some(ground) => {
                    navigation.segment_clear_with_budget(ground, position, allow_work)?
                }
                None => false,
            };
            from_ground
                || navigation
                    .connected_point_with_budget(home, [Some(position)], allow_work)?
                    .is_some()
        };
        if reachable {
            self.inner
                .ground_witnesses
                .lock()
                .unwrap()
                .entry((owner, cell))
                .or_insert(position);
        }
        Ok(reachable)
    }
}

impl SwarmReachabilityMap {
    /// Whether permanent terrain permits a valid approach to an interaction region.
    pub fn interaction_is_reachable(
        &self,
        owner: SwarmId,
        region: crate::nanobot::InteractionRegion,
    ) -> bool {
        self.interaction_is_reachable_with_budget(owner, region, &mut || true)
            .expect("unlimited interaction reachability query")
    }

    pub(crate) fn interaction_is_reachable_with_budget(
        &self,
        owner: SwarmId,
        region: crate::nanobot::InteractionRegion,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Result<bool, ()> {
        let Some(&home) = self.inner.homes.get(&owner) else {
            return Ok(false);
        };
        let navigation = &self.inner.navigation;
        Ok(region.contains(home)
            || navigation
                .connected_point_with_budget(
                    home,
                    navigation.interaction_probes(home, region),
                    allow_work,
                )?
                .is_some())
    }
}

fn tile_bounds(cell: IVec2) -> (Vec2, Vec2) {
    let min = cell.as_vec2() * ZONE_BLOCK_SIZE;
    let upper = (cell + IVec2::ONE).as_vec2();
    // Step inside in cell coordinates so rounding cannot cross the origin into the next tile.
    let max = Vec2::new(upper.x.next_down(), upper.y.next_down()) * ZONE_BLOCK_SIZE;
    (min, max)
}

/// Apply an owner-scoped gameplay intent edit through the scenario boundary.
///
/// Erases intentionally remain legal outside the reachable map so cleanup can remove
/// malformed or legacy intent. Only painting can create a prohibited Intent Zone.
pub fn apply_reachable_intent(
    grid: &mut IntentGrid,
    reachability: &SwarmReachabilityMap,
    owner: SwarmId,
    cell: IVec2,
    kind: IntentKind,
    action: IntentWriteAction,
) -> Result<bool, IntentWriteError> {
    let before = grid
        .cell(cell)
        .cloned()
        .ok_or(IntentWriteError::OutOfBounds)?;
    if action == IntentWriteAction::Paint && !reachability.contains(owner, cell, grid) {
        return Err(IntentWriteError::Unreachable);
    }
    match action {
        IntentWriteAction::Paint => grid.paint(cell, kind, owner),
        IntentWriteAction::Erase => grid.erase(cell, kind, owner),
    };
    Ok(before != *grid.cell(cell).expect("validated intent cell"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::Obstacle;

    #[test]
    fn permanent_wall_separates_reachable_intent_cells() {
        let grid = IntentGrid::new(8, 8);
        let map = SwarmReachabilityMap::new(
            &grid,
            [Obstacle::Rectangle {
                center: Vec2::new(0.0, 0.0),
                half: Vec2::new(36.0, 8_000.0),
            }],
            [(SwarmId::PLAYER, Vec2::new(-252.0, 36.0))],
        )
        .unwrap();

        assert!(map.contains(SwarmId::PLAYER, IVec2::new(-1, 0), &grid));
        assert!(!map.contains(SwarmId::PLAYER, IVec2::new(1, 0), &grid));
    }

    #[test]
    fn both_swarms_can_paint_a_split_tile_with_ground_on_their_side() {
        let mut grid = IntentGrid::new(8, 8);
        let left = SwarmId::PLAYER;
        let right = SwarmId(1);
        let left_point = Vec2::new(96.0, 256.0);
        let right_point = Vec2::new(416.0, 256.0);
        let tile = IVec2::ZERO;
        let map = SwarmReachabilityMap::new(
            &grid,
            [Obstacle::Rectangle {
                center: Vec2::new(256.0, 0.0),
                half: Vec2::new(36.0, 8_000.0),
            }],
            [(left, left_point), (right, right_point)],
        )
        .unwrap();

        for (owner, accessible, isolated) in [
            (left, left_point, right_point),
            (right, right_point, left_point),
        ] {
            let point = map.reachable_point(owner, tile, &grid).unwrap();
            assert_eq!(world_to_cell(point), tile);
            assert!(map.position_is_reachable(owner, accessible, &grid));
            assert!(!map.position_is_reachable(owner, isolated, &grid));
            let mut remaining = 16_usize;
            assert_eq!(
                map.position_is_reachable_with_budget(owner, isolated, &grid, &mut || {
                    let allowed = remaining > 0;
                    remaining = remaining.saturating_sub(1);
                    allowed
                }),
                Err(())
            );
            assert_eq!(
                apply_reachable_intent(
                    &mut grid,
                    &map,
                    owner,
                    tile,
                    IntentKind::Defend,
                    IntentWriteAction::Paint,
                ),
                Ok(true)
            );
        }
        let paint = grid.cell(tile).unwrap();
        assert!(paint.has_owned(IntentKind::Defend, left));
        assert!(paint.has_owned(IntentKind::Defend, right));
    }

    #[test]
    fn gameplay_paint_rejects_unreachable_cells_without_changing_the_grid() {
        let mut grid = IntentGrid::new(8, 8);
        let map = SwarmReachabilityMap::new(
            &grid,
            [Obstacle::Rectangle {
                center: Vec2::new(0.0, 0.0),
                half: Vec2::new(36.0, 8_000.0),
            }],
            [(SwarmId::PLAYER, Vec2::new(-252.0, 36.0))],
        )
        .unwrap();

        assert_eq!(
            apply_reachable_intent(
                &mut grid,
                &map,
                SwarmId::PLAYER,
                IVec2::new(1, 0),
                IntentKind::Defend,
                IntentWriteAction::Paint,
            ),
            Err(IntentWriteError::Unreachable)
        );
        assert!(grid.iter_active_cells().next().is_none());
    }
}
