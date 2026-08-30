//! Owner-scoped Swarm Tile and Threat projection.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use super::AllocationRegion;
use crate::{
    ZONE_BLOCK_SIZE,
    intent::IntentGrid,
    nanobot::{
        Health, Nanobot, NanobotType, OwnerSwarm, Structure, Swarm, SwarmId, SwarmMember,
        world_to_cell,
    },
    spatial::FixedSpatialBuckets,
};

/// Gameplay category used to rank and resolve a territory Threat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreatKind {
    DefenderNanobot,
    OtherNanobot,
    Structure,
}

/// Ordered response priority for one kind of territory Threat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThreatPriority {
    HostileDefender,
    HostileNanobot,
    HostileStructure,
}

/// Stable danger ordering for Defender response work.
pub const fn threat_danger_rank(kind: ThreatKind) -> ThreatPriority {
    match kind {
        ThreatKind::DefenderNanobot => ThreatPriority::HostileDefender,
        ThreatKind::OtherNanobot => ThreatPriority::HostileNanobot,
        ThreatKind::Structure => ThreatPriority::HostileStructure,
    }
}

/// A response changes targets only when newly available work is more dangerous.
pub fn higher_tier_threat_preempts(current: ThreatKind, candidate: ThreatKind) -> bool {
    threat_danger_rank(candidate) < threat_danger_rank(current)
}

/// Whether an existing response target is on a Swarm Tile or in its one-cell
/// Chebyshev Pursuit Halo.
pub fn pursuit_claim_is_spatially_valid(target: IVec2, territory_tiles: &[IVec2]) -> bool {
    territory_tiles
        .iter()
        .any(|tile| (target - *tile).abs().max_element() <= 1)
}

/// Stable entity identity and current physical state for one territory Threat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreatSnapshot {
    pub entity: Entity,
    pub cell: IVec2,
    pub position: Vec2,
    pub kind: ThreatKind,
}

#[derive(Debug, Clone, Copy)]
struct ThreatCandidate {
    swarm: SwarmId,
    snapshot: ThreatSnapshot,
}

#[derive(Debug, Default)]
struct TerritoryRegion {
    tiles: Vec<IVec2>,
    threats: Vec<ThreatSnapshot>,
}

#[derive(Debug, Default)]
struct SwarmTerritory {
    regions: BTreeMap<AllocationRegion, TerritoryRegion>,
    tile_count: u32,
    threat_count: u32,
}

/// Deterministic regional view of each swarm's current territory and Threats.
#[derive(Debug, Default, Resource)]
pub struct TerritorySnapshot {
    by_swarm: BTreeMap<SwarmId, SwarmTerritory>,
}

impl TerritorySnapshot {
    /// Swarms represented by the current projection, in stable identity order.
    pub fn swarms(&self) -> impl Iterator<Item = SwarmId> + '_ {
        self.by_swarm.keys().copied()
    }

    /// Number of unique Swarm Tiles claimed by `swarm`.
    pub fn tile_count(&self, swarm: SwarmId) -> u32 {
        self.by_swarm
            .get(&swarm)
            .map(|territory| territory.tile_count)
            .unwrap_or_default()
    }

    /// Number of active Threats physically occupying `swarm`'s tiles.
    pub fn threat_count(&self, swarm: SwarmId) -> u32 {
        self.by_swarm
            .get(&swarm)
            .map(|territory| territory.threat_count)
            .unwrap_or_default()
    }

    /// Allocation regions containing at least one of `swarm`'s tiles.
    pub fn regions(&self, swarm: SwarmId) -> impl Iterator<Item = AllocationRegion> + '_ {
        self.by_swarm
            .get(&swarm)
            .into_iter()
            .flat_map(|territory| territory.regions.keys().copied())
    }

    /// Swarm Tiles in one allocation region, in deterministic row-major order.
    pub fn tiles_in_region(&self, swarm: SwarmId, region: AllocationRegion) -> &[IVec2] {
        self.by_swarm
            .get(&swarm)
            .and_then(|territory| territory.regions.get(&region))
            .map_or(&[], |region| region.tiles.as_slice())
    }

    /// Threats on `swarm`'s tiles in one allocation region, ordered by tile and
    /// stable entity identity.
    pub fn threats_in_region(&self, swarm: SwarmId, region: AllocationRegion) -> &[ThreatSnapshot] {
        self.by_swarm
            .get(&swarm)
            .and_then(|territory| territory.regions.get(&region))
            .map_or(&[], |region| region.threats.as_slice())
    }

    /// Whether `cell` is one of `swarm`'s current Swarm Tiles.
    pub fn is_swarm_tile(&self, swarm: SwarmId, cell: IVec2) -> bool {
        self.tiles_in_region(swarm, AllocationRegion::for_cell(cell))
            .contains(&cell)
    }

    /// Whether an existing claim may continue pursuing a target in `cell`.
    pub fn pursuit_claim_is_spatially_valid(&self, swarm: SwarmId, cell: IVec2) -> bool {
        (-1..=1).any(|dy| (-1..=1).any(|dx| self.is_swarm_tile(swarm, cell + IVec2::new(dx, dy))))
    }
}

/// Defender capacity justified by peaceful territory reserve and active
/// Threats. Peaceful reserve is half the unique Swarm Tile count, rounded up.
pub(crate) fn defender_population_demand(tile_count: u32, threat_count: u32) -> u32 {
    tile_count.div_ceil(2).max(threat_count)
}

/// Rebuild the owner-scoped regional territory snapshot from current intent.
#[allow(clippy::type_complexity)]
pub fn project_territory_snapshot_system(
    grid: Res<IntentGrid>,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
    nanobots: Query<
        (
            Entity,
            &Transform,
            &SwarmMember,
            &NanobotType,
            Option<&Health>,
        ),
        With<Nanobot>,
    >,
    structures: Query<(Entity, &Transform, &OwnerSwarm, &Structure)>,
    mut snapshot: ResMut<TerritorySnapshot>,
) {
    snapshot.by_swarm.clear();
    let mut live_swarms = swarms
        .iter()
        .map(|(_, swarm)| *swarm)
        .collect::<BTreeSet<_>>();
    if live_swarms.is_empty() {
        live_swarms.insert(SwarmId::PLAYER);
    }

    let mut threats = FixedSpatialBuckets::new(ZONE_BLOCK_SIZE);
    for (entity, transform, member, kind, health) in &nanobots {
        if health.is_some_and(|health| health.current == 0) {
            continue;
        }
        let position = transform.translation.truncate();
        threats.insert(
            position,
            ThreatCandidate {
                swarm: member.0,
                snapshot: ThreatSnapshot {
                    entity,
                    cell: world_to_cell(position),
                    position,
                    kind: if *kind == NanobotType::Defender {
                        ThreatKind::DefenderNanobot
                    } else {
                        ThreatKind::OtherNanobot
                    },
                },
            },
        );
    }
    for (entity, transform, owner, structure) in &structures {
        if !structure.is_operational() {
            continue;
        }
        let Ok((_, swarm)) = swarms.get(owner.0) else {
            continue;
        };
        let position = transform.translation.truncate();
        threats.insert(
            position,
            ThreatCandidate {
                swarm: *swarm,
                snapshot: ThreatSnapshot {
                    entity,
                    cell: world_to_cell(position),
                    position,
                    kind: ThreatKind::Structure,
                },
            },
        );
    }
    threats.sort_entries_by(|left, right| {
        left.snapshot
            .entity
            .to_bits()
            .cmp(&right.snapshot.entity.to_bits())
    });

    for swarm in live_swarms {
        let tiles = grid.swarm_tiles(swarm);
        let tile_count = u32::try_from(tiles.len()).unwrap_or(u32::MAX);
        let mut territory = SwarmTerritory {
            tile_count,
            ..default()
        };
        for tile in tiles {
            territory
                .regions
                .entry(AllocationRegion::for_cell(tile))
                .or_default()
                .tiles
                .push(tile);
        }
        for region in territory.regions.values_mut() {
            for tile in &region.tiles {
                region.threats.extend(
                    threats
                        .entries(*tile)
                        .iter()
                        .filter(|candidate| candidate.swarm != swarm)
                        .map(|candidate| candidate.snapshot),
                );
            }
            territory.threat_count = territory
                .threat_count
                .saturating_add(u32::try_from(region.threats.len()).unwrap_or(u32::MAX));
        }
        snapshot.by_swarm.insert(swarm, territory);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ThreatKind, defender_population_demand, higher_tier_threat_preempts,
        pursuit_claim_is_spatially_valid, threat_danger_rank,
    };
    use bevy::prelude::IVec2;

    #[test]
    fn defender_population_demand_uses_reserve_until_threats_exceed_it() {
        let examples = [
            (0, 0, 0),
            (1, 0, 1),
            (2, 1, 1),
            (3, 1, 2),
            (4, 2, 2),
            (4, 3, 3),
        ];

        for (tile_count, threat_count, expected) in examples {
            assert_eq!(
                defender_population_demand(tile_count, threat_count),
                expected,
                "tile_count={tile_count}, threat_count={threat_count}",
            );
        }
    }

    #[test]
    fn threat_response_decisions_follow_danger_and_pursuit_boundaries() {
        assert!(
            threat_danger_rank(ThreatKind::DefenderNanobot)
                < threat_danger_rank(ThreatKind::OtherNanobot)
        );
        assert!(
            threat_danger_rank(ThreatKind::OtherNanobot)
                < threat_danger_rank(ThreatKind::Structure)
        );

        assert!(higher_tier_threat_preempts(
            ThreatKind::Structure,
            ThreatKind::OtherNanobot,
        ));
        assert!(!higher_tier_threat_preempts(
            ThreatKind::OtherNanobot,
            ThreatKind::OtherNanobot,
        ));
        assert!(!higher_tier_threat_preempts(
            ThreatKind::DefenderNanobot,
            ThreatKind::Structure,
        ));

        let territory = [IVec2::new(3, 4), IVec2::new(-2, 7)];
        for target in [
            IVec2::new(3, 4),
            IVec2::new(4, 4),
            IVec2::new(2, 4),
            IVec2::new(3, 5),
            IVec2::new(3, 3),
            IVec2::new(4, 5),
            IVec2::new(2, 3),
        ] {
            assert!(
                pursuit_claim_is_spatially_valid(target, &territory),
                "target {target:?} should remain within pursuit reach",
            );
        }
        assert!(!pursuit_claim_is_spatially_valid(
            IVec2::new(5, 4),
            &territory,
        ));
        assert!(!pursuit_claim_is_spatially_valid(
            IVec2::new(5, 6),
            &territory,
        ));
    }
}
