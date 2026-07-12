//! Regional work allocation and production ECS adapter.

pub mod allocator;
pub mod lease;
pub mod projection;
pub mod runtime;

use bevy::prelude::{Entity, IVec2};

use crate::nanobot::{PlannedKind, SwarmId};
use crate::resources::ResourceKind;

pub use allocator::*;
pub use lease::*;
pub use projection::{project_actionable_opportunities_system, ActionableProjection};
pub use runtime::*;

/// Intent cells per deterministic allocation region axis.
pub const ALLOCATION_REGION_CELLS: i32 = 8;

/// Stable allocation-region coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AllocationRegion {
    pub x: i32,
    pub y: i32,
}

impl AllocationRegion {
    pub fn for_cell(cell: IVec2) -> Self {
        Self {
            x: cell.x.div_euclid(ALLOCATION_REGION_CELLS),
            y: cell.y.div_euclid(ALLOCATION_REGION_CELLS),
        }
    }

    pub fn min_cell(self) -> IVec2 {
        IVec2::new(self.x, self.y) * ALLOCATION_REGION_CELLS
    }
}

/// Stable work-category order used by regional allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpportunityCategory {
    Gather,
    PlannedBuild,
    Maintenance,
    Defend,
    Haul,
}

impl OpportunityCategory {
    pub const COUNT: usize = 5;
    pub const ALL: [Self; Self::COUNT] = [
        Self::PlannedBuild,
        Self::Maintenance,
        Self::Gather,
        Self::Defend,
        Self::Haul,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Gather => 0,
            Self::PlannedBuild => 1,
            Self::Maintenance => 2,
            Self::Defend => 3,
            Self::Haul => 4,
        }
    }
}

/// Deterministic identity and exact target for actionable work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpportunityTarget {
    Gather {
        deposit: Entity,
        cell: IVec2,
    },
    PlannedBuild {
        structure: Entity,
        kind: PlannedKind,
    },
    Maintenance {
        structure: Entity,
    },
    Defend {
        cell: IVec2,
    },
    Haul {
        source: Entity,
        sink: Entity,
        kind: ResourceKind,
    },
}

/// Derived work visible to regional allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionableOpportunity {
    pub region: AllocationRegion,
    pub category: OpportunityCategory,
    pub target: OpportunityTarget,
    pub cell: IVec2,
    pub owner: Option<SwarmId>,
    pub paint_strength: u8,
    pub available_work: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_regions_floor_negative_cells_deterministically() {
        assert_eq!(
            AllocationRegion::for_cell(IVec2::new(0, 7)),
            AllocationRegion { x: 0, y: 0 }
        );
        assert_eq!(
            AllocationRegion::for_cell(IVec2::new(8, 0)),
            AllocationRegion { x: 1, y: 0 }
        );
        assert_eq!(
            AllocationRegion::for_cell(IVec2::new(-1, -8)),
            AllocationRegion { x: -1, y: -1 }
        );
        assert_eq!(
            AllocationRegion::for_cell(IVec2::new(-9, 0)),
            AllocationRegion { x: -2, y: 0 }
        );
    }
}
