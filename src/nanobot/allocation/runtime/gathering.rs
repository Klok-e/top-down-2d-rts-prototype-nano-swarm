use std::collections::{BTreeMap, BTreeSet};

use bevy::{ecs::system::SystemParam, prelude::*};

use super::super::{CategoryValues, RegionalPullBudget};
use super::{
    ActionableOpportunity, AllocationRegion, BotSnapshot, CandidateBounds, ClaimCounts,
    OpportunityCategory, OpportunityTarget, RUNTIME_MAX_CANDIDATE_REGIONS, RUNTIME_MAX_CANDIDATES,
    RegionalLease, allocation_candidate, choose_bounded_candidate_from_ordered_regions_with_claims,
    claims_for, region_distance_key,
};
use crate::{
    intent::IntentGrid,
    nanobot::{
        Cargo, CombatIdentity, GatherAssignment, InteractionRegion, LogisticsReservation, Nanobot,
        NanobotType, PlannedStructure, SpareGathering, Swarm, SwarmId, SwarmMember,
        gather::{
            DropOffArea, GatherStockpiles, eligible_drop_off, find_nearest_stockpile,
            painted_gather_regions,
        },
        gather_worker_slots,
        work_access::{WorkAccess, WorkReachability},
    },
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
    resources::ResourceDeposit,
    tuning::Tuning,
};

#[derive(SystemParam)]
#[allow(clippy::type_complexity)]
pub(super) struct GatheringInputs<'w, 's> {
    access: WorkAccess<'w, 's>,
    stockpiles: GatherStockpiles<'w, 's>,
    swarms: Query<'w, 's, &'static SwarmId, With<Swarm>>,
    identities: Query<'w, 's, &'static CombatIdentity>,
    deliveries: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static SwarmMember,
            &'static NanobotType,
            &'static Cargo,
            &'static LogisticsReservation,
        ),
        (
            With<Nanobot>,
            Without<SpareGathering>,
            Without<GatherAssignment>,
        ),
    >,
    pub(super) active: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static SwarmMember,
            Option<&'static GatherAssignment>,
            Option<&'static Cargo>,
            Option<&'static LogisticsReservation>,
            Has<SpareGathering>,
        ),
        (
            With<Nanobot>,
            Or<(With<GatherAssignment>, With<SpareGathering>)>,
        ),
    >,
}

#[derive(Clone, Copy)]
pub(super) struct GatherApproach {
    pub(super) cell: IVec2,
    pub(super) region: InteractionRegion,
    pub(super) endpoint: Vec2,
}

pub(super) struct RequiredGatherTrip {
    pub(super) worker: Entity,
    pub(super) swarm: SwarmId,
    pub(super) deposit: Entity,
    pub(super) origin: Vec2,
    pub(super) region: InteractionRegion,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn assign_spare_gatherers(
    commands: &mut Commands,
    tick: u64,
    tuning: &Tuning,
    navigation: &Navigation,
    grid: &IntentGrid,
    gathering: &GatheringInputs,
    worker_regions: &[(AllocationRegion, Vec<ActionableOpportunity>)],
    gather_open: &BTreeMap<SwarmId, u32>,
    claim_counts: &ClaimCounts,
    planned: &Query<(Entity, &mut PlannedStructure, &Transform)>,
    deposits: &Query<(&ResourceDeposit, &Transform)>,
    unused_workers: impl Iterator<Item = (BotSnapshot, Option<InteractionRegion>)>,
    mut required_trips: Vec<RequiredGatherTrip>,
    mut ring_claims: BTreeMap<Entity, usize>,
    mut expected_pickups: BTreeMap<Entity, u32>,
    mut expected_dropoffs: BTreeMap<Entity, u32>,
) {
    let unclaimed_required = worker_regions
        .iter()
        .flat_map(|(_, opportunities)| opportunities)
        .filter(|work| {
            work.category != OpportunityCategory::Gather
                && claims_for(claim_counts, **work) == 0
                && !matches!(work.target, OpportunityTarget::PlannedBuild { structure, .. }
                    if planned.get(structure).is_ok_and(|(_, plan, _)| plan.active_worker().is_some()))
        })
        .collect::<Vec<_>>();
    let mut required_staffed = BTreeMap::new();
    let unused_workers = unused_workers
        .filter(|(bot, _)| {
            gather_open.get(&bot.swarm) == Some(&0)
                && *required_staffed.entry(bot.swarm).or_insert_with(|| {
                    !unclaimed_required.iter().any(|work| {
                        work.owner.is_none_or(|owner| owner == bot.swarm)
                            && gathering.access.opportunity(bot.swarm, work.target)
                                != WorkReachability::Unreachable
                    })
                })
        })
        .collect::<Vec<_>>();
    if unused_workers.is_empty() {
        return;
    }
    let mut pending_required_routes = BTreeSet::new();
    for (transform, member, kind, cargo, claim) in &gathering.deliveries {
        if *kind != NanobotType::Worker || cargo.amount == 0 {
            continue;
        }
        let Ok((deposit, deposit_transform)) = deposits.get(claim.source) else {
            continue;
        };
        let position = transform.translation.truncate();
        if gathering.delivery_claim_covers(
            navigation,
            member.0,
            position,
            cargo,
            claim,
            &expected_dropoffs,
        ) {
            continue;
        }
        let (destination, pending) = gathering.drop_off(
            tuning,
            navigation,
            member.0,
            position,
            deposit,
            deposit_transform,
            cargo.amount,
            &expected_dropoffs,
        );
        if let Some((destination, _)) = destination {
            *expected_dropoffs.entry(destination).or_default() += cargo.amount;
        } else if pending {
            pending_required_routes.insert(member.0);
        }
    }

    required_trips.sort_unstable_by_key(|trip| {
        (
            crate::nanobot::combat_key(trip.worker, gathering.identities.get(trip.worker).ok()),
            trip.worker.to_bits(),
        )
    });
    // Mirror the ordinary arrival policy for required gatherers still on approach.
    for trip in required_trips {
        let Ok((deposit, transform)) = deposits.get(trip.deposit) else {
            continue;
        };
        let available = deposit.amount.saturating_sub(
            expected_pickups
                .get(&trip.deposit)
                .copied()
                .unwrap_or_default(),
        );
        if available == 0 {
            continue;
        }
        let position =
            match navigation.query_connectivity(trip.origin, RouteGoal::Interaction(trip.region)) {
                ConnectivityStatus::Connected { endpoint } => endpoint,
                ConnectivityStatus::Pending => {
                    pending_required_routes.insert(trip.swarm);
                    continue;
                }
                ConnectivityStatus::Unreachable => continue,
            };
        let (target, pending) = gathering.drop_off(
            tuning,
            navigation,
            trip.swarm,
            position,
            deposit,
            transform,
            1,
            &expected_dropoffs,
        );
        let Some((destination, room)) = target else {
            if pending {
                pending_required_routes.insert(trip.swarm);
            }
            continue;
        };
        let load = available
            .min(room)
            .min(tuning.logistics.worker_carry_capacity);
        *expected_pickups.entry(trip.deposit).or_default() += load;
        *expected_dropoffs.entry(destination).or_default() += load;
    }

    let gather_regions = worker_regions
        .iter()
        .filter_map(|(region, opportunities)| {
            let gather = opportunities
                .iter()
                .copied()
                .filter(|opportunity| opportunity.category == OpportunityCategory::Gather)
                .collect::<Vec<_>>();
            (!gather.is_empty()).then_some((*region, gather))
        })
        .collect::<Vec<_>>();
    let mut ordered_by_region = BTreeMap::new();
    let bounds = CandidateBounds {
        max_regions: RUNTIME_MAX_CANDIDATE_REGIONS,
        max_candidates: RUNTIME_MAX_CANDIDATES,
    };
    let capacity = tuning.logistics.worker_carry_capacity;
    for (bot, rejected) in unused_workers {
        if pending_required_routes.contains(&bot.swarm) {
            continue;
        }
        let ordered = ordered_by_region.entry(bot.region).or_insert_with(|| {
            let mut ordered = gather_regions
                .iter()
                .map(|(region, work)| (*region, work.as_slice()))
                .collect::<Vec<_>>();
            ordered.sort_by_key(|(region, _)| {
                (region_distance_key(bot.region, *region), region.y, region.x)
            });
            ordered
        });
        let pull = RegionalPullBudget {
            source_region: bot.region,
            categories: CategoryValues::new([1, 0, 0, 0]),
        };
        let mut trips = BTreeMap::new();
        let Some(decision) = choose_bounded_candidate_from_ordered_regions_with_claims(
            allocation_candidate(bot),
            pull,
            ordered.iter().copied(),
            bounds,
            |work| {
                let OpportunityTarget::Gather { deposit, cell } = work.target else {
                    return None;
                };
                let claims = ring_claims.get(&deposit).copied().unwrap_or_default();
                let (state, transform) = deposits.get(deposit).ok()?;
                let available = state
                    .amount
                    .saturating_sub(expected_pickups.get(&deposit).copied().unwrap_or_default());
                if claims >= gather_worker_slots(state.radius) as usize || available == 0 {
                    return None;
                }
                let approach = reachable_gather_approach(
                    bot, deposit, cell, deposits, grid, navigation, rejected,
                )?;
                let (destination, _) = gathering
                    .drop_off(
                        tuning,
                        navigation,
                        bot.swarm,
                        approach.endpoint,
                        state,
                        transform,
                        capacity,
                        &expected_dropoffs,
                    )
                    .0?;
                let mut claim = LogisticsReservation::new(
                    deposit,
                    destination,
                    state.kind,
                    capacity.min(available),
                );
                claim.destination_remaining = capacity;
                trips.insert(deposit, (approach, claim));
                Some(claims)
            },
        ) else {
            continue;
        };
        let work = decision.opportunity;
        let OpportunityTarget::Gather { deposit, .. } = work.target else {
            continue;
        };
        let (approach, claim) = trips[&deposit];
        commands.entity(bot.entity).insert((
            SpareGathering,
            GatherAssignment::from_paint(approach.cell, deposit),
            approach.region.movement_from(bot.position),
            claim,
            RegionalLease::new(
                work.region,
                work.category,
                work.target,
                work.owner,
                tick,
                0,
                tuning.autonomy.lease_no_progress_ttl_ticks,
            ),
        ));
        *ring_claims.entry(deposit).or_default() += 1;
        *expected_pickups.entry(deposit).or_default() += claim.source_remaining;
        *expected_dropoffs.entry(claim.destination).or_default() += claim.destination_remaining;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reachable_gather_approach(
    bot: BotSnapshot,
    deposit: Entity,
    cell: IVec2,
    deposits: &Query<(&ResourceDeposit, &Transform)>,
    grid: &crate::intent::IntentGrid,
    navigation: &Navigation,
    rejected: Option<InteractionRegion>,
) -> Option<GatherApproach> {
    let (state, transform) = deposits.get(deposit).ok()?;
    let check_approach = |cell, region| {
        if rejected == Some(region) {
            return None;
        }
        let ConnectivityStatus::Connected { endpoint } =
            navigation.query_connectivity(bot.position, RouteGoal::Interaction(region))
        else {
            return None;
        };
        Some(GatherApproach {
            cell,
            region,
            endpoint,
        })
    };
    let preferred = InteractionRegion::deposit(transform, state.radius).within_cell(cell);
    check_approach(cell, preferred).or_else(|| {
        painted_gather_regions(grid, bot.swarm, *transform, state.radius)
            .filter(|(candidate, _)| *candidate != cell)
            .filter_map(|(cell, region)| check_approach(cell, region))
            .min_by_key(|approach| (approach.cell.x, approach.cell.y))
    })
}

impl GatheringInputs<'_, '_> {
    fn delivery_claim_covers(
        &self,
        navigation: &Navigation,
        swarm: SwarmId,
        position: Vec2,
        cargo: &Cargo,
        claim: &LogisticsReservation,
        reserved: &BTreeMap<Entity, u32>,
    ) -> bool {
        if claim.destination_remaining < cargo.amount {
            return false;
        }
        self.stockpiles.get(claim.destination).is_ok_and(
            |(_, stockpile, transform, role, owner, condition)| {
                eligible_drop_off(
                    cargo.kind,
                    swarm,
                    stockpile,
                    transform,
                    role,
                    owner,
                    condition,
                    &self.swarms,
                    None,
                ) && stockpile.free_space()
                    >= reserved
                        .get(&claim.destination)
                        .copied()
                        .unwrap_or_default()
                    && navigation.query_connectivity(
                        position,
                        RouteGoal::Interaction(InteractionRegion::structure(transform)),
                    ) != ConnectivityStatus::Unreachable
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn drop_off(
        &self,
        tuning: &crate::tuning::Tuning,
        navigation: &Navigation,
        swarm: SwarmId,
        position: Vec2,
        deposit: &ResourceDeposit,
        transform: &Transform,
        min_room: u32,
        reserved: &BTreeMap<Entity, u32>,
    ) -> (Option<(Entity, u32)>, bool) {
        let mut pending = false;
        let target = find_nearest_stockpile(
            deposit.kind,
            position,
            swarm,
            min_room,
            None,
            &self.stockpiles,
            &self.swarms,
            &self.identities,
            Some(DropOffArea::for_deposit(
                transform.translation.truncate(),
                deposit.radius,
                tuning,
            )),
            |entity, stockpile| {
                let room = stockpile
                    .free_space()
                    .saturating_sub(reserved.get(&entity).copied().unwrap_or_default());
                if room < min_room { 0 } else { room }
            },
            |transform| match navigation.query_connectivity(
                position,
                RouteGoal::Interaction(InteractionRegion::structure(transform)),
            ) {
                ConnectivityStatus::Connected { .. } => true,
                ConnectivityStatus::Pending => {
                    pending = true;
                    false
                }
                ConnectivityStatus::Unreachable => false,
            },
        );
        (target, pending)
    }
}
