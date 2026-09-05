//! One interpretation of world footprints for routing, movement, and body placement.
use bevy::{ecs::system::SystemParam, prelude::*};

use crate::{
    ZONE_BLOCK_SIZE,
    intent::IntentGrid,
    nanobot::{
        Charger, PlannedStructure, ProductionFacility, Structure, StructureClearing,
        StructurePassage, structure_passage,
    },
    navigation::{Navigation, Obstacle},
    resources::{ResourceDeposit, Stockpile},
};

/// Reads lifecycle consequences without exposing their representation to consumers.
#[derive(SystemParam)]
#[allow(clippy::type_complexity)]
pub struct PhysicalWorld<'w, 's> {
    grid: Res<'w, IntentGrid>,
    objects: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            Option<&'static ResourceDeposit>,
            Option<&'static PlannedStructure>,
            Option<&'static StructureClearing>,
        ),
        Or<(
            With<ResourceDeposit>,
            With<Structure>,
            With<Stockpile>,
            With<ProductionFacility>,
            With<Charger>,
            With<PlannedStructure>,
            With<StructureClearing>,
        )>,
    >,
}

impl PhysicalWorld<'_, '_> {
    pub fn snapshot(&self) -> PhysicalGeometry {
        let mut objects: Vec<_> = self.objects.iter().collect();
        objects.sort_by_key(|(entity, ..)| entity.to_bits());
        let mut solids = Vec::new();
        let mut entry_barriers = Vec::new();
        for (_, transform, deposit, plan, clearing) in objects {
            if let Some(deposit) = deposit {
                solids.push(Obstacle::deposit(
                    transform.translation.truncate(),
                    deposit.radius,
                ));
                continue;
            }
            let shape = Obstacle::structure(transform);
            match structure_passage(plan, clearing) {
                StructurePassage::Traversable => {}
                StructurePassage::EntryBarred => entry_barriers.push(shape),
                StructurePassage::Solid => solids.push(shape),
            }
        }
        let min = IVec2::new(-(self.grid.width() / 2), -(self.grid.height() / 2)).as_vec2()
            * ZONE_BLOCK_SIZE;
        PhysicalGeometry {
            min,
            max: min
                + Vec2::new(self.grid.width() as f32, self.grid.height() as f32) * ZONE_BLOCK_SIZE,
            solids,
            entry_barriers,
        }
    }
}

/// Immutable spatial consequences at a simulation phase. Crowds remain local to consumers.
pub struct PhysicalGeometry {
    min: Vec2,
    max: Vec2,
    solids: Vec<Obstacle>,
    entry_barriers: Vec<Obstacle>,
}

impl PhysicalGeometry {
    fn in_bounds(&self, point: Vec2) -> bool {
        point.is_finite() && point.cmpge(self.min).all() && point.cmplt(self.max).all()
    }

    /// A new body cannot occupy a solid or an entry-barred footprint.
    pub fn can_occupy(&self, point: Vec2) -> bool {
        self.in_bounds(point)
            && self
                .solids
                .iter()
                .chain(&self.entry_barriers)
                .all(|shape| shape.admits_body(point))
    }

    /// Sweep a body against solids and entry barriers, allowing existing occupants to leave.
    pub fn movement_clear(&self, start: Vec2, end: Vec2) -> bool {
        self.in_bounds(start)
            && self.in_bounds(end)
            && self
                .solids
                .iter()
                .all(|shape| shape.segment_clear(start, end))
            && self
                .entry_barriers
                .iter()
                .all(|shape| !shape.admits_body(start) || shape.segment_clear(start, end))
    }

    pub(crate) fn refresh_navigation(self, navigation: &mut Navigation, grid: &IntentGrid) {
        navigation.refresh_clearing(self.entry_barriers);
        navigation.refresh(grid, self.solids);
    }
}
