//! Friendly interaction access across the combined construction commitment.

use super::{
    Charger, InteractionRegion, Nanobot, NanobotType, OwnerSwarm, PlannedKind, PlannedStructure,
    ProductionFacility, Structure, SwarmId, SwarmMember, gather::cell_overlaps_circle,
};
use crate::{
    intent::{IntentGrid, IntentKind},
    navigation::{AccessCheck, AccessStatus, Navigation, Obstacle, RoutePriority},
    resources::{ResourceDeposit, Stockpile},
};
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum AccessOwner {
    Shared,
    Swarm(SwarmId),
    Orphaned,
}

#[derive(Clone)]
struct AccessObject {
    entity: Entity,
    transform: Transform,
    shape: Obstacle,
    owner: AccessOwner,
    deposit: Option<ResourceDeposit>,
    planned: bool,
}

/// Snapshot includes plans as commitments, while its baseline contains only solids.
pub struct AccessLayout {
    objects: Vec<AccessObject>,
    builders: Vec<(SwarmId, Vec2)>,
    pending: std::cell::Cell<bool>,
}

impl AccessLayout {
    #[cfg(test)]
    fn accepts(
        &self,
        grid: &IntentGrid,
        swarm: SwarmId,
        candidate: &Transform,
        ignored_plan: Option<Entity>,
        builder: Option<Vec2>,
    ) -> bool {
        let navigation = Navigation::new(grid, vec![]);
        for _ in 0..1000 {
            match self.check(&navigation, grid, swarm, candidate, ignored_plan, builder) {
                AccessStatus::Accepted => return true,
                AccessStatus::Rejected => return false,
                AccessStatus::Pending => {
                    navigation.advance(grid, 32_768);
                }
            }
        }
        panic!("access check did not finish within its deterministic work allowance");
    }

    pub fn check(
        &self,
        navigation: &Navigation,
        grid: &IntentGrid,
        swarm: SwarmId,
        candidate: &Transform,
        ignored_plan: Option<Entity>,
        builder: Option<Vec2>,
    ) -> AccessStatus {
        use std::hash::{Hash, Hasher};
        let shape = Obstacle::structure(candidate);
        let Obstacle::Rectangle { center, half } = shape else {
            unreachable!()
        };
        if self
            .objects
            .iter()
            .filter(|object| Some(object.entity) != ignored_plan)
            .any(|object| object.shape.overlaps_rectangle(center, half, 0.0))
        {
            return AccessStatus::Rejected;
        }
        let builders: Vec<_> = builder.map_or_else(
            || {
                self.builders
                    .iter()
                    .filter(|(owner, _)| *owner == swarm)
                    .map(|(_, position)| *position)
                    .collect()
            },
            |position| vec![position],
        );
        if builders.is_empty() {
            return AccessStatus::Rejected;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.validation_key(grid).hash(&mut hasher);
        swarm.hash(&mut hasher);
        ignored_plan.hash(&mut hasher);
        [
            center.x.to_bits(),
            center.y.to_bits(),
            half.x.to_bits(),
            half.y.to_bits(),
        ]
        .hash(&mut hasher);
        navigation.query_access(
            hasher.finish(),
            &builders,
            || {
                let baseline = self
                    .objects
                    .iter()
                    .filter(|object| !object.planned)
                    .map(|object| object.shape)
                    .collect();
                let mut completed: Vec<_> = self
                    .objects
                    .iter()
                    .filter(|object| Some(object.entity) != ignored_plan)
                    .map(|object| object.shape)
                    .collect();
                completed.push(shape);
                let endpoints = self
                    .objects
                    .iter()
                    .filter(|object| {
                        Some(object.entity) != ignored_plan && self.protects(object, grid, swarm)
                    })
                    .map(|object| {
                        object.deposit.map_or_else(
                            || InteractionRegion::structure(&object.transform),
                            |deposit| InteractionRegion::deposit(&object.transform, deposit.radius),
                        )
                    })
                    .collect();
                AccessCheck {
                    baseline,
                    completed,
                    builders: builders.clone(),
                    target: InteractionRegion::structure(candidate),
                    endpoints,
                    swarm,
                }
            },
            if ignored_plan.is_some() {
                RoutePriority::Clearing
            } else {
                RoutePriority::Routine
            },
        )
    }

    fn protects(&self, object: &AccessObject, grid: &IntentGrid, swarm: SwarmId) -> bool {
        if let Some(deposit) = object.deposit {
            return deposit.amount > 0
                && grid.iter_active_cells().any(|(cell, paint)| {
                    paint.has(IntentKind::Gather)
                        && paint
                            .owner(IntentKind::Gather)
                            .is_none_or(|owner| owner == swarm)
                        && cell_overlaps_circle(
                            cell,
                            object.transform.translation.truncate(),
                            deposit.radius,
                        )
                });
        }
        matches!(object.owner, AccessOwner::Shared) || object.owner == AccessOwner::Swarm(swarm)
    }

    /// Include same-system deferred commitments in subsequent candidate checks.
    pub fn reserve(&mut self, swarm: SwarmId, transform: Transform) {
        self.objects.push(AccessObject {
            entity: Entity::PLACEHOLDER,
            transform,
            shape: Obstacle::structure(&transform),
            owner: AccessOwner::Swarm(swarm),
            deposit: None,
            planned: true,
        });
    }

    /// Invalidate a saved access decision when geometry, commitments, ownership,
    /// or Gather eligibility changes. Moving crowds do not change static access.
    pub fn validation_key(&self, grid: &IntentGrid) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.signature(None).hash(&mut hasher);
        (grid.width(), grid.height()).hash(&mut hasher);
        let mut eligibility: Vec<_> = self
            .objects
            .iter()
            .map(|object| {
                (
                    object.entity.to_bits(),
                    object.owner,
                    object.deposit.is_some_and(|deposit| deposit.amount > 0),
                )
            })
            .collect();
        eligibility.sort_unstable();
        eligibility.hash(&mut hasher);
        let mut gather: Vec<_> = grid
            .iter_active_cells()
            .filter(|(_, paint)| paint.has(IntentKind::Gather))
            .map(|(cell, paint)| (cell.x, cell.y, paint.owner(IntentKind::Gather)))
            .collect();
        gather.sort_unstable();
        gather.hash(&mut hasher);
        hasher.finish()
    }

    fn signature(&self, ignored: Option<Entity>) -> Vec<(u64, [u32; 4], bool)> {
        let mut result: Vec<_> = self
            .objects
            .iter()
            .filter(|object| Some(object.entity) != ignored)
            .map(|object| {
                let geometry = match object.shape {
                    Obstacle::Rectangle { center, half } => [
                        center.x.to_bits(),
                        center.y.to_bits(),
                        half.x.to_bits(),
                        half.y.to_bits(),
                    ],
                    Obstacle::Circle { center, radius } => [
                        center.x.to_bits(),
                        center.y.to_bits(),
                        radius.to_bits(),
                        u32::MAX,
                    ],
                };
                (object.entity.to_bits(), geometry, object.planned)
            })
            .collect();
        result.sort_unstable();
        result
    }
}

/// Query snapshot shared by placement and the completion access check.
#[derive(SystemParam)]
pub struct ConstructionAccess<'w, 's> {
    navigation: Res<'w, Navigation>,
    #[allow(clippy::type_complexity)]
    objects: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            Option<&'static ResourceDeposit>,
            Option<&'static PlannedStructure>,
            Option<&'static OwnerSwarm>,
        ),
        Or<(
            With<ResourceDeposit>,
            With<Structure>,
            With<Stockpile>,
            With<ProductionFacility>,
            With<Charger>,
            With<PlannedStructure>,
        )>,
    >,
    swarms: Query<'w, 's, &'static SwarmId>,
    workers: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static NanobotType,
            &'static SwarmMember,
        ),
        With<Nanobot>,
    >,
    cancelled: Option<Res<'w, CancelledSites>>,
}
impl ConstructionAccess<'_, '_> {
    pub fn snapshot(&self) -> AccessLayout {
        AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: self
                .objects
                .iter()
                .map(
                    |(entity, transform, deposit, planned, owner)| AccessObject {
                        entity,
                        transform: *transform,
                        shape: deposit.map_or_else(
                            || Obstacle::structure(transform),
                            |deposit| {
                                Obstacle::deposit(transform.translation.truncate(), deposit.radius)
                            },
                        ),
                        owner: owner.map_or(AccessOwner::Shared, |owner| {
                            self.swarms
                                .get(owner.0)
                                .copied()
                                .map_or(AccessOwner::Orphaned, AccessOwner::Swarm)
                        }),
                        deposit: deposit.copied(),
                        planned: planned.is_some(),
                    },
                )
                .collect(),
            builders: self
                .workers
                .iter()
                .filter(|(_, kind, _)| **kind == NanobotType::Worker)
                .map(|(transform, _, member)| (member.0, transform.translation.truncate()))
                .collect(),
        }
    }
    pub fn accepts(
        &self,
        layout: &AccessLayout,
        grid: &IntentGrid,
        swarm: SwarmId,
        kind: PlannedKind,
        position: Vec2,
    ) -> bool {
        if layout.pending.get() {
            return false;
        }
        let transform =
            crate::navigation::align_structure(Transform::from_translation(position.extend(0.0)));
        if self
            .cancelled
            .as_ref()
            .is_some_and(|sites| sites.excludes(layout, swarm, kind, &transform))
        {
            return false;
        }
        match layout.check(&self.navigation, grid, swarm, &transform, None, None) {
            AccessStatus::Accepted => true,
            AccessStatus::Rejected => false,
            AccessStatus::Pending => {
                layout.pending.set(true);
                false
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct CancelledSites {
    sites: Vec<CancelledSite>,
}
struct CancelledSite {
    entity: Entity,
    swarm: SwarmId,
    kind: PlannedKind,
    shape: Obstacle,
    signature: Vec<(u64, [u32; 4], bool)>,
}
impl CancelledSites {
    pub fn record(
        &mut self,
        layout: &AccessLayout,
        cancelled: Entity,
        swarm: SwarmId,
        kind: PlannedKind,
        transform: &Transform,
    ) {
        let cancelled_ids: std::collections::HashSet<_> = self
            .sites
            .iter()
            .map(|site| site.entity.to_bits())
            .chain([cancelled.to_bits()])
            .collect();
        let mut signature = layout.signature(Some(cancelled));
        signature.retain(|(entity, _, _)| !cancelled_ids.contains(entity));
        for site in &mut self.sites {
            site.signature
                .retain(|(entity, _, _)| !cancelled_ids.contains(entity));
        }
        self.sites.retain(|site| site.signature == signature);
        self.sites.push(CancelledSite {
            entity: cancelled,
            swarm,
            kind,
            shape: Obstacle::structure(transform),
            signature,
        });
    }
    pub fn excludes(
        &self,
        layout: &AccessLayout,
        swarm: SwarmId,
        kind: PlannedKind,
        transform: &Transform,
    ) -> bool {
        let shape = Obstacle::structure(transform);
        self.sites.iter().any(|site| {
            site.swarm == swarm
                && site.kind == kind
                && site.shape == shape
                && site.signature == layout.signature(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn structure(
        index: u32,
        center: Vec2,
        half: Vec2,
        owner: Option<SwarmId>,
        planned: bool,
    ) -> AccessObject {
        let transform =
            Transform::from_translation(center.extend(0.0)).with_scale((half / 32.0).extend(1.0));
        AccessObject {
            entity: Entity::from_bits(index as u64),
            transform,
            shape: Obstacle::structure(&transform),
            owner: owner.map_or(AccessOwner::Shared, AccessOwner::Swarm),
            deposit: None,
            planned,
        }
    }
    #[test]
    fn closing_last_passage_rejects_site_between_connected_friendly_terminals() {
        // A vertical wall has a 144-unit opening; one 72-unit site in its middle
        // leaves two 36-unit gaps, neither admitting a 68-unit body.
        let layout = AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: vec![
                structure(
                    1,
                    Vec2::new(36.0, -292.0),
                    Vec2::new(36.0, 220.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    2,
                    Vec2::new(36.0, 292.0),
                    Vec2::new(36.0, 220.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    3,
                    Vec2::new(-252.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
                structure(
                    4,
                    Vec2::new(324.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
            ],
            builders: vec![(SwarmId::PLAYER, Vec2::new(-108.0, 0.0))],
        };
        let candidate = Transform::from_xyz(36.0, 0.0, 0.0).with_scale(Vec3::splat(72.0 / 64.0));
        assert!(!layout.accepts(
            &IntentGrid::new(2, 2),
            SwarmId::PLAYER,
            &candidate,
            None,
            None
        ));
    }
    #[test]
    fn disconnected_friendly_networks_need_not_be_joined_by_new_construction() {
        let layout = AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: vec![
                structure(
                    1,
                    Vec2::new(36.0, 0.0),
                    Vec2::new(36.0, 512.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    2,
                    Vec2::new(-324.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
                structure(
                    3,
                    Vec2::new(324.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
            ],
            builders: vec![(SwarmId::PLAYER, Vec2::new(-108.0, 0.0))],
        };
        let candidate = crate::navigation::align_structure(Transform::from_xyz(-180.0, 180.0, 0.0));
        assert!(layout.accepts(
            &IntentGrid::new(2, 2),
            SwarmId::PLAYER,
            &candidate,
            None,
            None
        ));
    }

    #[test]
    fn individually_open_plans_cannot_jointly_close_the_friendly_passage() {
        let mut layout = AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: vec![
                structure(
                    1,
                    Vec2::new(36.0, -310.0),
                    Vec2::new(36.0, 202.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    2,
                    Vec2::new(36.0, 310.0),
                    Vec2::new(36.0, 202.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    3,
                    Vec2::new(-252.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
                structure(
                    4,
                    Vec2::new(324.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
            ],
            builders: vec![(SwarmId::PLAYER, Vec2::new(-108.0, 0.0))],
        };
        let candidate = crate::navigation::align_structure(Transform::from_xyz(36.0, 36.0, 0.0));
        let grid = IntentGrid::new(2, 2);
        assert!(layout.accepts(&grid, SwarmId::PLAYER, &candidate, None, None));
        layout.objects.push(structure(
            5,
            Vec2::new(36.0, -36.0),
            Vec2::splat(36.0),
            Some(SwarmId::PLAYER),
            true,
        ));
        assert!(!layout.accepts(&grid, SwarmId::PLAYER, &candidate, None, None));
        // Enemy terminals receive no friendly-access guarantee.
        for object in &mut layout.objects {
            object.owner = AccessOwner::Swarm(SwarmId(2));
        }
        assert!(layout.accepts(&grid, SwarmId::PLAYER, &candidate, None, None));
    }

    #[test]
    fn only_gather_eligible_deposits_receive_connection_protection() {
        let mut layout = AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: vec![
                structure(
                    1,
                    Vec2::new(36.0, -292.0),
                    Vec2::new(36.0, 220.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    2,
                    Vec2::new(36.0, 292.0),
                    Vec2::new(36.0, 220.0),
                    Some(SwarmId(2)),
                    false,
                ),
                structure(
                    3,
                    Vec2::new(-252.0, 0.0),
                    Vec2::splat(36.0),
                    Some(SwarmId::PLAYER),
                    false,
                ),
            ],
            builders: vec![(SwarmId::PLAYER, Vec2::new(-108.0, 0.0))],
        };
        let transform = Transform::from_xyz(324.0, 0.0, 0.0);
        layout.objects.push(AccessObject {
            entity: Entity::from_bits(4),
            transform,
            shape: Obstacle::deposit(Vec2::new(324.0, 0.0), 36.0),
            owner: AccessOwner::Shared,
            deposit: Some(ResourceDeposit {
                kind: crate::resources::ResourceKind::Minerals,
                amount: 20,
                capacity: 20,
                radius: 36.0,
            }),
            planned: false,
        });
        let candidate = Transform::from_xyz(36.0, 0.0, 0.0).with_scale(Vec3::splat(72.0 / 64.0));
        let mut grid = IntentGrid::new(2, 2);
        assert!(layout.accepts(&grid, SwarmId::PLAYER, &candidate, None, None));
        grid.paint(IVec2::ZERO, IntentKind::Gather);
        assert!(!layout.accepts(&grid, SwarmId::PLAYER, &candidate, None, None));
        layout
            .objects
            .last_mut()
            .unwrap()
            .deposit
            .as_mut()
            .unwrap()
            .amount = 0;
        assert!(layout.accepts(&grid, SwarmId::PLAYER, &candidate, None, None));
    }

    #[test]
    fn a_builder_in_another_disconnected_network_cannot_service_the_site() {
        let layout = AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: vec![structure(
                1,
                Vec2::new(36.0, 0.0),
                Vec2::new(36.0, 512.0),
                Some(SwarmId(2)),
                false,
            )],
            builders: vec![(SwarmId::PLAYER, Vec2::new(-108.0, 0.0))],
        };
        let candidate = crate::navigation::align_structure(Transform::from_xyz(252.0, 0.0, 0.0));
        assert!(!layout.accepts(
            &IntentGrid::new(2, 2),
            SwarmId::PLAYER,
            &candidate,
            None,
            None
        ));
    }

    #[test]
    fn cancelled_site_stays_excluded_after_its_own_removal_until_layout_changes() {
        let cancelled = structure(
            1,
            Vec2::new(36.0, 36.0),
            Vec2::splat(36.0),
            Some(SwarmId::PLAYER),
            true,
        );
        let candidate = cancelled.transform;
        let mut layout = AccessLayout {
            pending: std::cell::Cell::new(false),
            objects: vec![cancelled],
            builders: vec![],
        };
        let mut sites = CancelledSites::default();
        sites.record(
            &layout,
            Entity::from_bits(1),
            SwarmId::PLAYER,
            PlannedKind::Charger,
            &candidate,
        );
        layout.objects.clear();
        assert!(sites.excludes(&layout, SwarmId::PLAYER, PlannedKind::Charger, &candidate));
        layout.objects.push(structure(
            2,
            Vec2::new(324.0, 36.0),
            Vec2::splat(36.0),
            Some(SwarmId::PLAYER),
            false,
        ));
        assert!(!sites.excludes(&layout, SwarmId::PLAYER, PlannedKind::Charger, &candidate));
    }
}
