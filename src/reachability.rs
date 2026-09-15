//! Scenario-stable ownership-aware intent bounds.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use bevy::prelude::{IVec2, Resource, Vec2};

use crate::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::SwarmId,
    navigation::{Navigation, Obstacle, RouteOutcome},
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
    cells: Mutex<HashMap<(SwarmId, IVec2), bool>>,
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
            }),
        })
    }

    /// Whether `owner` can route from its scenario home to `cell`'s center.
    pub fn contains(&self, owner: SwarmId, cell: IVec2, grid: &IntentGrid) -> bool {
        if !grid.in_bounds(cell) {
            return false;
        }
        let Some(&home) = self.inner.homes.get(&owner) else {
            return false;
        };
        if let Some(&reachable) = self.inner.cells.lock().unwrap().get(&(owner, cell)) {
            return reachable;
        }
        let center = (cell.as_vec2() + Vec2::splat(0.5)) * ZONE_BLOCK_SIZE;
        let reachable = matches!(
            self.inner
                .navigation
                .route(home, center, grid, owner, false),
            RouteOutcome::Found(_)
        );
        self.inner
            .cells
            .lock()
            .unwrap()
            .insert((owner, cell), reachable);
        reachable
    }
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
