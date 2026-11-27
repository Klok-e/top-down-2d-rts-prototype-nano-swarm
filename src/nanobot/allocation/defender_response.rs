//! Territory-wide Defender response reconciliation under regional allocation.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};

use bevy::{ecs::entity::EntityHashMap, math::FloatOrd, prelude::*};

use super::{
    AllocationRegion, CandidateBounds, RUNTIME_MAX_CANDIDATE_REGIONS, RUNTIME_MAX_CANDIDATES,
    TerritorySnapshot, ThreatKind, ThreatPriority, ThreatSnapshot, higher_tier_threat_preempts,
    threat_danger_rank,
};
use crate::nanobot::{
    ChargerAssignment, ChargerProgress, CombatIdentity, Commitment, DefenderDisengagement,
    DefenderRout, DirectMovementComponent, Health, Nanobot, NanobotType, OwnerSwarm, RegionalLease,
    Structure, Swarm, SwarmId, SwarmMember, combat_key, world_to_cell,
};
use crate::{intent::IntentGrid, reachability::SwarmReachabilityMap};

/// One explicit pursuit claim on one hostile entity.
#[derive(Debug, Clone, Copy, Component, PartialEq, Eq)]
pub struct DefenderResponse {
    pub target: Entity,
}

#[derive(Debug, Clone, Copy)]
struct DefenderState {
    entity: Entity,
    combat_key: u64,
    position: Vec2,
    region: AllocationRegion,
    swarm: SwarmId,
    available: bool,
    previous_response: Option<DefenderResponse>,
    target: Option<ThreatSnapshot>,
}

#[derive(Debug, Clone, Copy)]
struct LiveTarget {
    swarm: SwarmId,
    snapshot: ThreatSnapshot,
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

/// Empty and saturated regions leave the candidate bounds.
#[derive(Debug, Default)]
struct ResponseWorkIndex {
    by_swarm: BTreeMap<SwarmId, ThreatsByTier>,
    occurrences: EntityHashMap<Vec<ThreatOccurrence>>,
    claims: EntityHashMap<usize>,
    defender_capacity: usize,
}

impl ResponseWorkIndex {
    fn is_empty(&self) -> bool {
        self.by_swarm.values().all(BTreeMap::is_empty)
    }

    fn from_territory(
        territory: &TerritorySnapshot,
        claims: EntityHashMap<usize>,
        defender_capacity: usize,
    ) -> Self {
        let mut index = Self {
            claims,
            defender_capacity,
            ..default()
        };
        for swarm in territory.swarms() {
            for region in territory.regions(swarm) {
                for threat in territory.threats_in_region(swarm, region) {
                    index.add_occurrence(swarm, region, *threat);
                }
            }
        }
        index
    }

    fn add_occurrence(
        &mut self,
        swarm: SwarmId,
        region: AllocationRegion,
        snapshot: ThreatSnapshot,
    ) {
        let key = (snapshot.cell.y, snapshot.cell.x, snapshot.combat_key);
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
        // A preemption can reopen work in a swarm whose Threats are all saturated.
        self.by_swarm.entry(swarm).or_default();
        if self
            .claims
            .get(&snapshot.entity)
            .copied()
            .unwrap_or_default()
            < response_capacity(snapshot.kind, self.defender_capacity)
        {
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

    fn claim(&mut self, threat: ThreatSnapshot) -> bool {
        let count = self.claims.entry(threat.entity).or_default();
        *count += 1;
        if *count < response_capacity(threat.kind, self.defender_capacity) {
            return false;
        }
        let occurrences = &self.occurrences[&threat.entity];
        for occurrence in occurrences {
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
            }
        }
        true
    }

    fn release(&mut self, threat: ThreatSnapshot) -> Vec<SwarmId> {
        let count = self.claims.get_mut(&threat.entity).unwrap();
        *count -= 1;
        let mut affected_swarms = Vec::new();
        if *count + 1 == response_capacity(threat.kind, self.defender_capacity)
            && let Some(occurrences) = self.occurrences.get(&threat.entity).cloned()
        {
            for occurrence in occurrences {
                affected_swarms.push(occurrence.swarm);
                self.insert_occurrence(occurrence);
            }
        }
        affected_swarms
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
}

fn region_distance(left: AllocationRegion, right: AllocationRegion) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y)
}

fn response_capacity(kind: ThreatKind, defender_capacity: usize) -> usize {
    match kind {
        ThreatKind::DefenderNanobot => defender_capacity,
        ThreatKind::OtherNanobot | ThreatKind::Structure => 1,
    }
}

fn response_bounds() -> CandidateBounds {
    CandidateBounds {
        max_regions: RUNTIME_MAX_CANDIDATE_REGIONS,
        max_candidates: RUNTIME_MAX_CANDIDATES,
    }
}

#[derive(Clone, Copy)]
enum AssignmentPhase {
    Available,
    Preemption,
}

impl AssignmentPhase {
    fn includes(self, state: &DefenderState) -> bool {
        state.available
            && match self {
                Self::Available => state.target.is_none(),
                Self::Preemption => state
                    .target
                    .is_some_and(|target| target.kind != ThreatKind::DefenderNanobot),
            }
    }

    fn permits(self, state: &DefenderState, kind: ThreatKind) -> bool {
        match self {
            Self::Available => state.target.is_none(),
            Self::Preemption => state
                .target
                .is_some_and(|target| higher_tier_threat_preempts(target.kind, kind)),
        }
    }
}

type PairRank = (ThreatPriority, u32, FloatOrd, u64, u64);

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ResponsePair {
    rank: PairRank,
    defender: usize,
    group: usize,
    target: Entity,
}

fn pair_rank(
    defender_key: u64,
    position: Vec2,
    region: AllocationRegion,
    target: ThreatSnapshot,
) -> PairRank {
    (
        threat_danger_rank(target.kind),
        region_distance(region, AllocationRegion::for_cell(target.cell)),
        FloatOrd(position.distance_squared(target.position)),
        defender_key,
        target.combat_key,
    )
}

#[derive(Default)]
struct ResponseGroup {
    defenders: Vec<usize>,
    candidates: EntityHashMap<ThreatSnapshot>,
}

impl ResponseGroup {
    fn best_pair(
        &self,
        group: usize,
        states: &[DefenderState],
        phase: AssignmentPhase,
        target: ThreatSnapshot,
    ) -> Option<Reverse<ResponsePair>> {
        let defender = self
            .defenders
            .iter()
            .copied()
            .filter(|&index| phase.permits(&states[index], target.kind))
            .min_by_key(|&index| {
                let state = &states[index];
                (
                    FloatOrd(state.position.distance_squared(target.position)),
                    state.combat_key,
                )
            })?;
        let state = &states[defender];
        Some(Reverse(ResponsePair {
            rank: pair_rank(state.combat_key, state.position, state.region, target),
            defender,
            group,
            target: target.entity,
        }))
    }

    fn refresh(
        &mut self,
        group: usize,
        states: &[DefenderState],
        phase: AssignmentPhase,
        work: &ResponseWorkIndex,
        bounds: CandidateBounds,
        pending: &mut BinaryHeap<Reverse<ResponsePair>>,
    ) {
        let source = &states[self.defenders[0]];
        let page = work.bounded(source.swarm, source.region, bounds);
        let mut candidates = EntityHashMap::with_capacity(page.len());
        for target in page {
            candidates.insert(target.entity, target);
            if !self.candidates.contains_key(&target.entity) {
                pending.extend(self.best_pair(group, states, phase, target));
            }
        }
        self.candidates = candidates;
    }
}

fn assign_responses(
    states: &mut [DefenderState],
    work: &mut ResponseWorkIndex,
    phase: AssignmentPhase,
    bounds: CandidateBounds,
) {
    let mut by_region = BTreeMap::<_, ResponseGroup>::new();
    for (index, state) in states
        .iter()
        .enumerate()
        .filter(|(_, state)| phase.includes(state))
    {
        if work.by_swarm.contains_key(&state.swarm) {
            by_region
                .entry((state.swarm, state.region))
                .or_default()
                .defenders
                .push(index);
        }
    }
    let mut groups = by_region.into_values().collect::<Vec<_>>();
    let mut pending = BinaryHeap::new();
    for (index, group) in groups.iter_mut().enumerate() {
        group.refresh(index, states, phase, work, bounds, &mut pending);
    }

    // Defenders in the same region share a bounded page. Its best pair per
    // target can only worsen as Defenders gain or upgrade their assignments.
    while let Some(Reverse(pair)) = pending.pop() {
        let target_entity = pair.target;
        let group = &groups[pair.group];
        let Some(&target) = group.candidates.get(&target_entity) else {
            continue;
        };
        if !phase.permits(&states[pair.defender], target.kind) {
            pending.extend(group.best_pair(pair.group, states, phase, target));
            continue;
        }

        let previous = states[pair.defender].target.replace(target);
        let saturated = work.claim(target);
        let released_swarms = previous.map_or_else(Vec::new, |previous| work.release(previous));
        if saturated || !released_swarms.is_empty() {
            for (index, group) in groups.iter_mut().enumerate() {
                if (saturated && group.candidates.contains_key(&target_entity))
                    || released_swarms.contains(&states[group.defenders[0]].swarm)
                {
                    group.refresh(index, states, phase, work, bounds, &mut pending);
                }
            }
        }
        if !saturated {
            pending.extend(groups[pair.group].best_pair(pair.group, states, phase, target));
        }
    }
}

/// Reconcile valid pursuit claims, movement destinations, uncovered Threats,
/// and strict higher-tier preemption once per fixed simulation step.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn reconcile_defender_responses_system(
    tuning: Res<crate::tuning::Tuning>,
    grid: Res<IntentGrid>,
    reachability: Option<Res<SwarmReachabilityMap>>,
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
            Option<&CombatIdentity>,
            Has<DefenderDisengagement>,
            Has<DefenderRout>,
        ),
        With<Nanobot>,
    >,
    nanobot_targets: Query<
        (
            &Transform,
            &NanobotType,
            &SwarmMember,
            Option<&Health>,
            Option<&CombatIdentity>,
        ),
        With<Nanobot>,
    >,
    structure_targets: Query<(&Transform, &OwnerSwarm, &Structure, Option<&CombatIdentity>)>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut response_movement: Query<&mut DirectMovementComponent, With<DefenderResponse>>,
) {
    let live_target = |entity: Entity| -> Option<LiveTarget> {
        if let Ok((transform, kind, member, health, identity)) = nanobot_targets.get(entity) {
            if health.is_some_and(|health| health.current == 0) {
                return None;
            }
            let position = transform.translation.truncate();
            return Some(LiveTarget {
                swarm: member.0,
                snapshot: ThreatSnapshot {
                    entity,
                    combat_key: combat_key(entity, identity),
                    cell: world_to_cell(position),
                    position,
                    kind: if *kind == NanobotType::Defender {
                        ThreatKind::DefenderNanobot
                    } else {
                        ThreatKind::OtherNanobot
                    },
                },
            });
        }
        let (transform, owner, structure, identity) = structure_targets.get(entity).ok()?;
        if structure.is_collapsed() {
            return None;
        }
        let position = transform.translation.truncate();
        Some(LiveTarget {
            swarm: *swarms.get(owner.0).ok()?,
            snapshot: ThreatSnapshot {
                entity,
                combat_key: combat_key(entity, identity),
                cell: world_to_cell(position),
                position,
                kind: ThreatKind::Structure,
            },
        })
    };

    let unsorted = defenders
        .iter()
        .filter(|(_, _, kind, ..)| **kind == NanobotType::Defender)
        .map(
            |(
                entity,
                transform,
                _,
                member,
                commitment,
                health,
                response,
                assignment,
                progress,
                identity,
                disengaged,
                routing,
            )| {
                let position = transform.translation.truncate();
                DefenderState {
                    entity,
                    combat_key: combat_key(entity, identity),
                    position,
                    region: AllocationRegion::for_cell(world_to_cell(position)),
                    swarm: member.0,
                    previous_response: response.copied(),
                    target: None,
                    available: health.is_none_or(|health| health.current > 0)
                        && *commitment == Commitment::Idle
                        && assignment.is_none()
                        && progress.is_none()
                        && !disengaged
                        && !routing,
                }
            },
        )
        .collect::<Vec<_>>();
    // A stable sort by combat key: the query index breaks ties. Sorting the small
    // pairs and gathering once avoids moving whole states through a merge sort.
    let mut order = unsorted
        .iter()
        .enumerate()
        .map(|(index, state)| (state.combat_key, index as u32))
        .collect::<Vec<_>>();
    order.sort_unstable();
    let mut states = order
        .into_iter()
        .map(|(_, index)| unsorted[index as usize])
        .collect::<Vec<_>>();

    let capacity = usize::from(tuning.combat.responders_per_defender_threat);
    let mut claims = EntityHashMap::default();
    for state in states.iter_mut().filter(|state| state.available) {
        let Some(response) = state.previous_response else {
            continue;
        };
        let Some(target) = live_target(response.target) else {
            continue;
        };
        if target.swarm == state.swarm
            || !territory.pursuit_claim_is_spatially_valid(state.swarm, target.snapshot.cell)
            || reachability.as_ref().is_some_and(|map| {
                !map.position_is_reachable(state.swarm, target.snapshot.position, &grid)
            })
        {
            continue;
        }
        let count = claims.entry(response.target).or_default();
        if *count < response_capacity(target.snapshot.kind, capacity) {
            *count += 1;
            state.target = Some(target.snapshot);
        }
    }

    if states.iter().any(|state| {
        territory.threat_count(state.swarm) > 0
            && (AssignmentPhase::Available.includes(state)
                || AssignmentPhase::Preemption.includes(state))
    }) {
        let mut work = ResponseWorkIndex::from_territory(&territory, claims, capacity);
        if !work.is_empty() {
            let bounds = response_bounds();
            assign_responses(&mut states, &mut work, AssignmentPhase::Available, bounds);
            assign_responses(&mut states, &mut work, AssignmentPhase::Preemption, bounds);
        }
    }

    for state in states {
        let Some(target) = state.target else {
            if state.previous_response.is_some() {
                commands
                    .entity(state.entity)
                    .remove::<(DefenderResponse, DirectMovementComponent)>();
            }
            continue;
        };
        let response = DefenderResponse {
            target: target.entity,
        };
        let movement = DirectMovementComponent {
            speed: None,
            interaction: None,
            xy: target.position,
            stop_radius: tuning.combat.attack_range,
        };
        if state.previous_response == Some(response) {
            if let Ok(mut current) = response_movement.get_mut(state.entity) {
                *current = movement;
            } else {
                commands.entity(state.entity).insert(movement);
            }
        } else {
            commands
                .entity(state.entity)
                .remove::<RegionalLease>()
                .insert((response, movement));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn choose_bounded_response_from_regions<'a>(
        defender_position: Vec2,
        defender_region: AllocationRegion,
        regions: impl IntoIterator<Item = (AllocationRegion, &'a [ThreatSnapshot])>,
        claimed: &BTreeSet<Entity>,
        bounds: CandidateBounds,
    ) -> Option<ThreatSnapshot> {
        let mut index = ResponseWorkIndex {
            claims: claimed.iter().map(|&target| (target, 1)).collect(),
            defender_capacity: 1,
            ..default()
        };
        for (region, threats) in regions {
            for threat in threats {
                index.add_occurrence(SwarmId::PLAYER, region, *threat);
            }
        }
        index
            .bounded(SwarmId::PLAYER, defender_region, bounds)
            .into_iter()
            .min_by_key(|target| pair_rank(0, defender_position, defender_region, *target))
    }

    fn threat(id: u64, cell: IVec2, position: Vec2, kind: ThreatKind) -> ThreatSnapshot {
        ThreatSnapshot {
            entity: Entity::from_bits(id),
            combat_key: id,
            cell,
            position,
            kind,
        }
    }

    #[test]
    fn bounded_response_choice_uses_danger_and_combat_identity() {
        let source = AllocationRegion { x: 0, y: 0 };
        let local = [
            ThreatSnapshot {
                combat_key: 1,
                ..threat(
                    7,
                    IVec2::ZERO,
                    Vec2::new(20.0, 0.0),
                    ThreatKind::OtherNanobot,
                )
            },
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

        for max_candidates in [1, 2] {
            let chosen = choose_bounded_response_from_regions(
                Vec2::ZERO,
                source,
                [(source, local.as_slice())],
                &BTreeSet::new(),
                CandidateBounds {
                    max_candidates,
                    ..bounds
                },
            )
            .expect("same-tier Threats are available");
            assert_eq!(chosen.entity, Entity::from_bits(7));
        }

        let claimed = BTreeSet::from([Entity::from_bits(7)]);
        let chosen = choose_bounded_response_from_regions(
            Vec2::ZERO,
            source,
            [(source, local.as_slice())],
            &claimed,
            bounds,
        )
        .expect("the unclaimed same-tier Threat remains available");
        assert_eq!(chosen.entity, Entity::from_bits(3));
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
    fn preemption_chooses_nearest_lower_tier_response_then_combat_identity() {
        let candidate = threat(90, IVec2::ZERO, Vec2::ZERO, ThreatKind::DefenderNanobot);
        let state = |defender, combat_key, position, current_kind| DefenderState {
            entity: Entity::from_bits(defender),
            combat_key,
            position,
            region: AllocationRegion::for_cell(IVec2::ZERO),
            swarm: SwarmId::PLAYER,
            available: true,
            previous_response: None,
            target: Some(threat(
                100 + defender,
                IVec2::ZERO,
                Vec2::ZERO,
                current_kind,
            )),
        };
        let mut states = [
            state(10, 1, Vec2::new(100.0, 0.0), ThreatKind::Structure),
            state(30, 2, Vec2::new(10.0, 0.0), ThreatKind::Structure),
            state(15, 3, Vec2::new(10.0, 0.0), ThreatKind::Structure),
            state(20, 4, Vec2::ZERO, ThreatKind::DefenderNanobot),
        ];
        let mut work = ResponseWorkIndex {
            claims: states
                .iter()
                .map(|state| (state.target.unwrap().entity, 1))
                .collect(),
            defender_capacity: 1,
            ..default()
        };
        work.add_occurrence(SwarmId::PLAYER, states[0].region, candidate);

        assign_responses(
            &mut states,
            &mut work,
            AssignmentPhase::Preemption,
            response_bounds(),
        );

        let selected = states
            .iter()
            .find(|state| state.target == Some(candidate))
            .expect("an eligible response should preempt to the higher-tier threat");
        assert_eq!(selected.entity, Entity::from_bits(30));
    }
}
