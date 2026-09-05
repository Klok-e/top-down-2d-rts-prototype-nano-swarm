//! Territory-wide Defender response reconciliation under regional allocation.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use bevy::{
    ecs::entity::{EntityHashMap, EntityHashSet},
    prelude::*,
};

use super::{
    AllocationRegion, CandidateBounds, RUNTIME_MAX_CANDIDATE_REGIONS, RUNTIME_MAX_CANDIDATES,
    TerritorySnapshot, ThreatKind, ThreatPriority, ThreatSnapshot, higher_tier_threat_preempts,
    threat_danger_rank,
};
use crate::nanobot::{
    ChargerAssignment, ChargerProgress, Commitment, DEFENDER_ATTACK_RANGE, DirectMovementComponent,
    Health, Nanobot, NanobotType, OwnerSwarm, RegionalLease, Structure, Swarm, SwarmId,
    SwarmMember, world_to_cell,
};

/// One explicit pursuit claim on one hostile entity.
#[derive(Debug, Clone, Copy, Component, PartialEq, Eq)]
pub struct DefenderResponse {
    pub target: Entity,
}

#[derive(Debug, Clone, Copy)]
struct DefenderState {
    entity: Entity,
    position: Vec2,
    region: AllocationRegion,
    swarm: SwarmId,
    response: Option<DefenderResponse>,
    available: bool,
}

#[derive(Debug, Clone, Copy)]
struct ActiveResponse {
    defender: Entity,
    position: Vec2,
    region: AllocationRegion,
    swarm: SwarmId,
    response: DefenderResponse,
    threat_kind: ThreatKind,
}

#[derive(Debug, Clone, Copy)]
struct LiveTarget {
    position: Vec2,
    cell: IVec2,
    swarm: SwarmId,
    kind: ThreatKind,
}

type ThreatOrderKey = (i32, i32, u64);
type ThreatsByRegion = BTreeMap<AllocationRegion, BTreeMap<ThreatOrderKey, ThreatSnapshot>>;
type ThreatsByTier = BTreeMap<ThreatPriority, ThreatsByRegion>;

#[derive(Debug, Clone, Copy)]
struct ThreatOccurrence {
    swarm: SwarmId,
    tier: ThreatPriority,
    region: AllocationRegion,
    key: ThreatOrderKey,
    snapshot: ThreatSnapshot,
}

/// Per-step view containing only Threat work not already claimed by a
/// Defender. Empty and exhausted regions are removed as claims are acquired.
#[derive(Debug, Default)]
struct ResponseWorkIndex {
    by_swarm: BTreeMap<SwarmId, ThreatsByTier>,
    occurrences: EntityHashMap<Vec<ThreatOccurrence>>,
}

impl ResponseWorkIndex {
    fn is_empty(&self) -> bool {
        self.by_swarm.is_empty()
    }

    fn from_territory(territory: &TerritorySnapshot, claimed: &BTreeSet<Entity>) -> Self {
        let mut index = Self::default();
        for swarm in territory.swarms() {
            for region in territory.regions(swarm) {
                for threat in territory.threats_in_region(swarm, region) {
                    index.add_occurrence(swarm, region, *threat, claimed);
                }
            }
        }
        index
    }

    fn from_regions<'a>(
        swarm: SwarmId,
        regions: impl IntoIterator<Item = (AllocationRegion, &'a [ThreatSnapshot])>,
        claimed: &BTreeSet<Entity>,
    ) -> Self {
        let mut index = Self::default();
        for (region, threats) in regions {
            for threat in threats {
                index.add_occurrence(swarm, region, *threat, claimed);
            }
        }
        index
    }

    fn add_occurrence(
        &mut self,
        swarm: SwarmId,
        region: AllocationRegion,
        snapshot: ThreatSnapshot,
        claimed: &BTreeSet<Entity>,
    ) {
        let key = (snapshot.cell.y, snapshot.cell.x, snapshot.entity.to_bits());
        let occurrence = ThreatOccurrence {
            swarm,
            tier: threat_danger_rank(snapshot.kind),
            region,
            key,
            snapshot,
        };
        self.occurrences
            .entry(snapshot.entity)
            .or_default()
            .push(occurrence);
        if !claimed.contains(&snapshot.entity) {
            self.insert_occurrence(occurrence);
        }
    }

    fn insert_occurrence(&mut self, occurrence: ThreatOccurrence) {
        self.by_swarm
            .entry(occurrence.swarm)
            .or_default()
            .entry(occurrence.tier)
            .or_default()
            .entry(occurrence.region)
            .or_default()
            .insert(occurrence.key, occurrence.snapshot);
    }

    fn claim(&mut self, entity: Entity) {
        let Some(occurrences) = self.occurrences.get(&entity).cloned() else {
            return;
        };
        for occurrence in occurrences {
            let mut remove_swarm = false;
            if let Some(tiers) = self.by_swarm.get_mut(&occurrence.swarm) {
                let mut remove_tier = false;
                if let Some(regions) = tiers.get_mut(&occurrence.tier) {
                    let remove_region =
                        regions.get_mut(&occurrence.region).is_some_and(|threats| {
                            threats.remove(&occurrence.key);
                            threats.is_empty()
                        });
                    if remove_region {
                        regions.remove(&occurrence.region);
                    }
                    remove_tier = regions.is_empty();
                }
                if remove_tier {
                    tiers.remove(&occurrence.tier);
                }
                remove_swarm = tiers.is_empty();
            }
            if remove_swarm {
                self.by_swarm.remove(&occurrence.swarm);
            }
        }
    }

    fn release(&mut self, entity: Entity) {
        let Some(occurrences) = self.occurrences.get(&entity).cloned() else {
            return;
        };
        for occurrence in occurrences {
            self.insert_occurrence(occurrence);
        }
    }

    fn bounded(
        &self,
        swarm: SwarmId,
        source_region: AllocationRegion,
        bounds: CandidateBounds,
    ) -> Vec<ThreatSnapshot> {
        if bounds.max_regions == 0 || bounds.max_candidates == 0 {
            return Vec::new();
        }
        let Some(tiers) = self.by_swarm.get(&swarm) else {
            return Vec::new();
        };
        let Some((_, regions)) = tiers.first_key_value() else {
            return Vec::new();
        };
        let mut ordered_regions = regions.keys().copied().collect::<Vec<_>>();
        ordered_regions
            .sort_by_key(|region| (region_distance(source_region, *region), region.y, region.x));
        let mut candidates = Vec::new();
        for region in ordered_regions.into_iter().take(bounds.max_regions) {
            let remaining = bounds.max_candidates.saturating_sub(candidates.len());
            candidates.extend(regions[&region].values().copied().take(remaining));
            if candidates.len() >= bounds.max_candidates {
                break;
            }
        }
        candidates
    }

    fn choose(
        &self,
        swarm: SwarmId,
        defender_position: Vec2,
        defender_region: AllocationRegion,
        bounds: CandidateBounds,
    ) -> Option<ThreatSnapshot> {
        choose_response_target(
            defender_position,
            defender_region,
            self.bounded(swarm, defender_region, bounds),
        )
    }
}

fn region_distance(left: AllocationRegion, right: AllocationRegion) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y)
}

fn choose_response_target(
    defender_position: Vec2,
    defender_region: AllocationRegion,
    candidates: impl IntoIterator<Item = ThreatSnapshot>,
) -> Option<ThreatSnapshot> {
    candidates.into_iter().min_by(|left, right| {
        threat_danger_rank(left.kind)
            .cmp(&threat_danger_rank(right.kind))
            .then_with(|| {
                region_distance(defender_region, AllocationRegion::for_cell(left.cell)).cmp(
                    &region_distance(defender_region, AllocationRegion::for_cell(right.cell)),
                )
            })
            .then_with(|| {
                defender_position
                    .distance_squared(left.position)
                    .total_cmp(&defender_position.distance_squared(right.position))
            })
            .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
    })
}

/// Choose the most dangerous bounded unclaimed Threat, then prefer regional
/// and physical proximity before stable entity identity.
pub fn choose_bounded_response_target(
    territory: &TerritorySnapshot,
    swarm: SwarmId,
    defender_position: Vec2,
    defender_region: AllocationRegion,
    claimed: &BTreeSet<Entity>,
    bounds: CandidateBounds,
) -> Option<ThreatSnapshot> {
    ResponseWorkIndex::from_territory(territory, claimed).choose(
        swarm,
        defender_position,
        defender_region,
        bounds,
    )
}

/// Pure bounded decision seam for tests and non-ECS allocation adapters.
pub fn choose_bounded_response_from_regions<'a>(
    defender_position: Vec2,
    defender_region: AllocationRegion,
    regions: impl IntoIterator<Item = (AllocationRegion, &'a [ThreatSnapshot])>,
    claimed: &BTreeSet<Entity>,
    bounds: CandidateBounds,
) -> Option<ThreatSnapshot> {
    ResponseWorkIndex::from_regions(SwarmId::PLAYER, regions, claimed).choose(
        SwarmId::PLAYER,
        defender_position,
        defender_region,
        bounds,
    )
}

fn response_bounds() -> CandidateBounds {
    CandidateBounds {
        max_regions: RUNTIME_MAX_CANDIDATE_REGIONS,
        max_candidates: RUNTIME_MAX_CANDIDATES,
    }
}

fn has_unclaimed_threats(territory: &TerritorySnapshot, claimed: &EntityHashSet) -> bool {
    territory.swarms().any(|swarm| {
        territory.regions(swarm).any(|region| {
            territory
                .threats_in_region(swarm, region)
                .iter()
                .any(|threat| !claimed.contains(&threat.entity))
        })
    })
}

/// One possible higher-tier response replacement considered by reconciliation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsePreemptionOption {
    pub defender: Entity,
    pub defender_position: Vec2,
    pub defender_region: AllocationRegion,
    pub current_kind: ThreatKind,
    pub candidate: ThreatSnapshot,
}

/// One deterministic higher-tier replacement decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsePreemptionDecision {
    pub defender: Entity,
    pub target: ThreatSnapshot,
}

fn preemption_order(option: ResponsePreemptionOption) -> (ThreatPriority, u32, f32, u64, u64) {
    (
        threat_danger_rank(option.candidate.kind),
        region_distance(
            option.defender_region,
            AllocationRegion::for_cell(option.candidate.cell),
        ),
        option
            .defender_position
            .distance_squared(option.candidate.position),
        option.candidate.entity.to_bits(),
        option.defender.to_bits(),
    )
}

/// Choose the most dangerous uncovered target, then the nearest lower-tier
/// response, with stable target and Defender identities as final tie-breakers.
pub fn choose_response_preemption(
    options: impl IntoIterator<Item = ResponsePreemptionOption>,
) -> Option<ResponsePreemptionDecision> {
    options
        .into_iter()
        .filter(|option| higher_tier_threat_preempts(option.current_kind, option.candidate.kind))
        .min_by(|left, right| compare_preemption(preemption_order(*left), preemption_order(*right)))
        .map(|option| ResponsePreemptionDecision {
            defender: option.defender,
            target: option.candidate,
        })
}

/// Reconcile valid pursuit claims, movement destinations, uncovered Threats,
/// and strict higher-tier preemption once per fixed simulation step.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn reconcile_defender_responses_system(
    mut commands: Commands,
    territory: Res<TerritorySnapshot>,
    defenders: Query<
        (
            Entity,
            &Transform,
            &NanobotType,
            &SwarmMember,
            &Commitment,
            Option<&Health>,
            Option<&DefenderResponse>,
            Option<&ChargerAssignment>,
            Option<&ChargerProgress>,
        ),
        With<Nanobot>,
    >,
    nanobot_targets: Query<
        (&Transform, &NanobotType, &SwarmMember, Option<&Health>),
        With<Nanobot>,
    >,
    structure_targets: Query<(&Transform, &OwnerSwarm, &Structure)>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut response_movement: Query<&mut DirectMovementComponent, With<DefenderResponse>>,
) {
    let live_target = |entity: Entity| -> Option<LiveTarget> {
        if let Ok((transform, kind, member, health)) = nanobot_targets.get(entity) {
            if health.is_some_and(|health| health.current == 0) {
                return None;
            }
            let position = transform.translation.truncate();
            return Some(LiveTarget {
                position,
                cell: world_to_cell(position),
                swarm: member.0,
                kind: if *kind == NanobotType::Defender {
                    ThreatKind::DefenderNanobot
                } else {
                    ThreatKind::OtherNanobot
                },
            });
        }
        let (transform, owner, structure) = structure_targets.get(entity).ok()?;
        if !structure.is_operational() {
            return None;
        }
        let position = transform.translation.truncate();
        Some(LiveTarget {
            position,
            cell: world_to_cell(position),
            swarm: *swarms.get(owner.0).ok()?,
            kind: ThreatKind::Structure,
        })
    };

    let mut states = defenders
        .iter()
        .filter(|(_, _, kind, _, _, _, _, _, _)| **kind == NanobotType::Defender)
        .map(
            |(entity, transform, _, member, commitment, health, response, assignment, progress)| {
                let position = transform.translation.truncate();
                DefenderState {
                    entity,
                    position,
                    region: AllocationRegion::for_cell(world_to_cell(position)),
                    swarm: member.0,
                    response: response.copied(),
                    available: health.is_none_or(|health| health.current > 0)
                        && *commitment == Commitment::Idle
                        && assignment.is_none()
                        && progress.is_none(),
                }
            },
        )
        .collect::<Vec<_>>();
    states.sort_by_key(|state| state.entity.to_bits());

    let mut claimed = EntityHashSet::default();
    let mut active = Vec::new();
    for state in &mut states {
        let Some(response) = state.response else {
            continue;
        };
        let target = live_target(response.target);
        let valid = state.available
            && target.is_some_and(|target| {
                target.swarm != state.swarm
                    && territory.pursuit_claim_is_spatially_valid(state.swarm, target.cell)
                    && claimed.insert(response.target)
            });
        let Some(target) = target.filter(|_| valid) else {
            commands
                .entity(state.entity)
                .remove::<DefenderResponse>()
                .remove::<DirectMovementComponent>();
            state.response = None;
            continue;
        };
        if let Ok(mut movement) = response_movement.get_mut(state.entity) {
            movement.xy = target.position;
            movement.stop_radius = DEFENDER_ATTACK_RANGE;
            movement.interaction = None;
            movement.speed = None;
        } else {
            commands
                .entity(state.entity)
                .insert(DirectMovementComponent {
                    speed: None,
                    interaction: None,
                    xy: target.position,
                    stop_radius: DEFENDER_ATTACK_RANGE,
                });
        }
        active.push(ActiveResponse {
            defender: state.entity,
            position: state.position,
            region: state.region,
            swarm: state.swarm,
            response,
            threat_kind: target.kind,
        });
    }

    if !has_unclaimed_threats(&territory, &claimed) {
        return;
    }

    let ordered_claimed = claimed.iter().copied().collect::<BTreeSet<_>>();
    let mut work = ResponseWorkIndex::from_territory(&territory, &ordered_claimed);
    if work.is_empty() {
        return;
    }

    for state in states
        .iter_mut()
        .filter(|state| state.available && state.response.is_none())
    {
        let Some(target) =
            work.choose(state.swarm, state.position, state.region, response_bounds())
        else {
            continue;
        };
        let response = DefenderResponse {
            target: target.entity,
        };
        commands
            .entity(state.entity)
            .remove::<RegionalLease>()
            .insert((
                response,
                DirectMovementComponent {
                    speed: None,
                    interaction: None,
                    xy: target.position,
                    stop_radius: DEFENDER_ATTACK_RANGE,
                },
            ));
        state.response = Some(response);
        claimed.insert(target.entity);
        work.claim(target.entity);
        active.push(ActiveResponse {
            defender: state.entity,
            position: state.position,
            region: state.region,
            swarm: state.swarm,
            response,
            threat_kind: target.kind,
        });
    }

    for _ in 0..active.len().saturating_mul(2) {
        let options = active.iter().copied().flat_map(|response| {
            work.bounded(response.swarm, response.region, response_bounds())
                .into_iter()
                .map(move |candidate| ResponsePreemptionOption {
                    defender: response.defender,
                    defender_position: response.position,
                    defender_region: response.region,
                    current_kind: response.threat_kind,
                    candidate,
                })
        });
        let Some(decision) = choose_response_preemption(options) else {
            break;
        };
        let index = active
            .iter()
            .position(|response| response.defender == decision.defender)
            .expect("preemption decision references an active response");
        let target = decision.target;
        let response = &mut active[index];
        claimed.remove(&response.response.target);
        work.release(response.response.target);
        claimed.insert(target.entity);
        work.claim(target.entity);
        response.response = DefenderResponse {
            target: target.entity,
        };
        response.threat_kind = target.kind;
        commands.entity(response.defender).insert(response.response);
        if let Ok(mut movement) = response_movement.get_mut(response.defender) {
            movement.xy = target.position;
            movement.stop_radius = DEFENDER_ATTACK_RANGE;
            movement.interaction = None;
            movement.speed = None;
        } else {
            commands
                .entity(response.defender)
                .insert(DirectMovementComponent {
                    speed: None,
                    interaction: None,
                    xy: target.position,
                    stop_radius: DEFENDER_ATTACK_RANGE,
                });
        }
    }
}

fn compare_preemption(
    left: (ThreatPriority, u32, f32, u64, u64),
    right: (ThreatPriority, u32, f32, u64, u64),
) -> Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.total_cmp(&right.2))
        .then_with(|| left.3.cmp(&right.3))
        .then_with(|| left.4.cmp(&right.4))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn threat(id: u64, cell: IVec2, position: Vec2, kind: ThreatKind) -> ThreatSnapshot {
        ThreatSnapshot {
            entity: Entity::from_bits(id),
            cell,
            position,
            kind,
        }
    }

    #[test]
    fn bounded_response_choice_uses_danger_proximity_and_stable_identity() {
        let source = AllocationRegion { x: 0, y: 0 };
        let local = [
            threat(
                7,
                IVec2::ZERO,
                Vec2::new(20.0, 0.0),
                ThreatKind::OtherNanobot,
            ),
            threat(
                3,
                IVec2::ZERO,
                Vec2::new(20.0, 0.0),
                ThreatKind::OtherNanobot,
            ),
        ];
        let dangerous = [threat(
            9,
            IVec2::new(8, 0),
            Vec2::new(900.0, 0.0),
            ThreatKind::DefenderNanobot,
        )];
        let bounds = CandidateBounds {
            max_regions: 2,
            max_candidates: 3,
        };

        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            [
                (AllocationRegion { x: 1, y: 0 }, dangerous.as_slice()),
                (source, local.as_slice()),
            ],
            &BTreeSet::new(),
            bounds,
        )
        .expect("a bounded Threat is available");
        assert_eq!(chosen.entity, Entity::from_bits(9));

        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            [(source, local.as_slice())],
            &BTreeSet::new(),
            bounds,
        )
        .expect("same-tier Threats are available");
        assert_eq!(chosen.entity, Entity::from_bits(3));

        let claimed = BTreeSet::from([Entity::from_bits(3)]);
        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            [(source, local.as_slice())],
            &claimed,
            bounds,
        )
        .expect("the unclaimed same-tier Threat remains available");
        assert_eq!(chosen.entity, Entity::from_bits(7));
    }

    #[test]
    fn bounded_response_choice_pages_past_claimed_candidates_and_empty_regions() {
        let source = AllocationRegion { x: 0, y: 0 };
        let threats = (1..=129)
            .map(|id| {
                threat(
                    id,
                    IVec2::ZERO,
                    Vec2::new(id as f32, 0.0),
                    ThreatKind::OtherNanobot,
                )
            })
            .collect::<Vec<_>>();
        let claimed = (1..=128).map(Entity::from_bits).collect::<BTreeSet<_>>();
        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            [(source, threats.as_slice())],
            &claimed,
            CandidateBounds {
                max_regions: 1,
                max_candidates: 128,
            },
        )
        .expect("claimed candidates must not consume the bounded page");
        assert_eq!(chosen.entity, Entity::from_bits(129));

        let empty = [];
        let distant = [threat(
            200,
            IVec2::new(128, 0),
            Vec2::new(65_536.0, 0.0),
            ThreatKind::Structure,
        )];
        let mut regions = (0..16)
            .map(|x| (AllocationRegion { x, y: 0 }, empty.as_slice()))
            .collect::<Vec<_>>();
        regions.push((AllocationRegion { x: 16, y: 0 }, distant.as_slice()));
        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            regions,
            &BTreeSet::new(),
            CandidateBounds {
                max_regions: 16,
                max_candidates: 128,
            },
        )
        .expect("empty territory regions must not consume the work-region bound");
        assert_eq!(chosen.entity, Entity::from_bits(200));
    }

    #[test]
    fn danger_tier_precedes_candidate_and_region_bounds() {
        let source = AllocationRegion { x: 0, y: 0 };
        let mut crowded_region = (1..=128)
            .map(|id| {
                threat(
                    id,
                    IVec2::ZERO,
                    Vec2::new(id as f32, 0.0),
                    ThreatKind::Structure,
                )
            })
            .collect::<Vec<_>>();
        crowded_region.push(threat(
            500,
            IVec2::ZERO,
            Vec2::new(500.0, 0.0),
            ThreatKind::DefenderNanobot,
        ));
        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            [(source, crowded_region.as_slice())],
            &BTreeSet::new(),
            CandidateBounds {
                max_regions: 1,
                max_candidates: 128,
            },
        )
        .expect("the highest danger tier must be selected before candidate bounding");
        assert_eq!(chosen.entity, Entity::from_bits(500));

        let nearby_structures = (0..16)
            .map(|x| {
                (
                    AllocationRegion { x, y: 0 },
                    [threat(
                        600 + x as u64,
                        IVec2::new(x * 8, 0),
                        Vec2::new((x * 4_096) as f32, 0.0),
                        ThreatKind::Structure,
                    )],
                )
            })
            .collect::<Vec<_>>();
        let distant_defender = [threat(
            700,
            IVec2::new(128, 0),
            Vec2::new(65_536.0, 0.0),
            ThreatKind::DefenderNanobot,
        )];
        let regions = nearby_structures
            .iter()
            .map(|(region, threats)| (*region, threats.as_slice()))
            .chain(std::iter::once((
                AllocationRegion { x: 16, y: 0 },
                distant_defender.as_slice(),
            )))
            .collect::<Vec<_>>();
        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            regions,
            &BTreeSet::new(),
            CandidateBounds {
                max_regions: 16,
                max_candidates: 128,
            },
        )
        .expect("the highest danger tier must be selected before region bounding");
        assert_eq!(chosen.entity, Entity::from_bits(700));
    }

    #[test]
    fn preemption_chooses_nearest_lower_tier_response_then_stable_identity() {
        let candidate = threat(90, IVec2::ZERO, Vec2::ZERO, ThreatKind::DefenderNanobot);
        let option = |defender, position| ResponsePreemptionOption {
            defender: Entity::from_bits(defender),
            defender_position: position,
            defender_region: AllocationRegion::for_cell(IVec2::ZERO),
            current_kind: ThreatKind::Structure,
            candidate,
        };
        let decision = choose_response_preemption([
            option(10, Vec2::new(100.0, 0.0)),
            option(20, Vec2::new(10.0, 0.0)),
        ])
        .expect("a higher-tier candidate should preempt one response");
        assert_eq!(decision.defender, Entity::from_bits(20));

        let decision = choose_response_preemption([
            option(30, Vec2::new(10.0, 0.0)),
            option(15, Vec2::new(10.0, 0.0)),
        ])
        .expect("stable identity should break an exact responder tie");
        assert_eq!(decision.defender, Entity::from_bits(15));

        let same_tier = ResponsePreemptionOption {
            current_kind: ThreatKind::DefenderNanobot,
            ..option(10, Vec2::ZERO)
        };
        assert_eq!(choose_response_preemption([same_tier]), None);
    }
}
