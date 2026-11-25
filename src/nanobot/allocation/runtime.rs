//! Production ECS adapter for deterministic regional allocation.

use std::{
    cmp::{Ordering, Reverse},
    collections::BTreeMap,
};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{
    ActionableOpportunity, ActionableProjection, AllocationCandidate, AllocationClock,
    AllocationRegion, CandidateBounds, CategoryEligibility, CategoryWeights, OpportunityCategory,
    OpportunityTarget, RegionalLease, choose_bounded_candidate_from_ordered_regions_with_claims,
    outward_pull_budgets, pressure_map,
};
use crate::{
    nanobot::{
        Commitment, DirectMovementComponent, ExtractProgress, GatherAssignment, GatherQuotas,
        HaulerAssignment, HaulerLoad, HaulerLoading, Health, InteractionRegion,
        LogisticsReservation, MaintenanceAssignment, MaintenanceProgress, Nanobot, NanobotType,
        PlannedStructure, PlannedStructureClaim, PlannedStructureProgress, ProductionFacility,
        ReturningToStockpile, SwarmId, SwarmMember, WorkerLoad,
        charge::{Charge, Charger, ChargerAssignment, ChargerProgress, minerals_to_fully_charge},
        gather::painted_gather_regions,
        gather_worker_slots,
        logistics_leg::{DestinationRole, PickupBuffer},
    },
    navigation::{ConnectivityStatus, Navigation, RouteGoal},
    resources::{ResourceDeposit, Stockpile, StockpileRole},
};

/// Maximum projection buckets examined for one nanobot acquisition.
pub const RUNTIME_MAX_CANDIDATE_REGIONS: usize = 16;
/// Maximum exact opportunities examined for one nanobot acquisition.
pub const RUNTIME_MAX_CANDIDATES: usize = 128;

/// Deterministic ordering: promoted starvation debt first, then nearest-region order.
pub fn region_fairness_sort_key(
    source: AllocationRegion,
    region: AllocationRegion,
    age: u32,
    promotion_ticks: u32,
) -> (bool, Reverse<u32>, u32, i32, i32) {
    (
        age < promotion_ticks,
        Reverse(age),
        region_distance_key(source, region),
        region.y,
        region.x,
    )
}

#[derive(Debug, Default, Resource)]
pub struct RegionalServiceAges {
    waiting: BTreeMap<(SwarmId, AllocationRegion, usize), u32>,
}

/// Waiting age for terminal consumers with actionable Logistics Legs.
#[derive(Debug, Default, Resource)]
pub struct TerminalDemandAges {
    waiting: BTreeMap<Entity, u32>,
}

impl TerminalDemandAges {
    pub fn waiting_ticks(&self, terminal: Entity) -> u32 {
        self.waiting.get(&terminal).copied().unwrap_or_default()
    }
}

/// Runtime ordering points exposed to category lifecycle plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum RegionalAllocationSet {
    Project,
    Invalidate,
    Acquire,
}

#[derive(Debug, Resource)]
pub(super) struct AllocationTickDue {
    due: bool,
    initialized: bool,
}

/// One-shot request for acquisition on the current fixed step.
#[derive(Debug, Default, Resource)]
pub struct RegionalAllocationWake {
    requested: bool,
}

impl RegionalAllocationWake {
    /// Ensure acquisition runs after the current projection/invalidation pass.
    pub fn request_current_pass(&mut self) {
        self.requested = true;
    }

    fn take(&mut self) -> bool {
        std::mem::take(&mut self.requested)
    }
}

impl Default for AllocationTickDue {
    fn default() -> Self {
        Self {
            due: true,
            initialized: false,
        }
    }
}

/// Single production allocator. Projection and invalidation run every fixed tick;
/// new claims run on deterministic 100 ms simulation-time boundaries.
pub struct RegionalAllocationPlugin;

impl Plugin for RegionalAllocationPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::nanobot::CombatIdentityPlugin>() {
            app.add_plugins(crate::nanobot::CombatIdentityPlugin);
        }
        app.init_resource::<crate::tuning::Tuning>()
            .init_resource::<ActionableProjection>()
            .init_resource::<super::TerritorySnapshot>()
            .init_resource::<super::defender_staging::DefenderStagingLayouts>()
            .init_resource::<AllocationClock>()
            .init_resource::<AllocationTickDue>()
            .init_resource::<RegionalAllocationWake>()
            .init_resource::<TerminalDemandAges>()
            .init_resource::<RegionalServiceAges>()
            .configure_sets(
                FixedUpdate,
                (
                    RegionalAllocationSet::Project,
                    RegionalAllocationSet::Invalidate,
                    RegionalAllocationSet::Acquire,
                )
                    .chain(),
            )
            .add_systems(
                FixedUpdate,
                (
                    super::project_actionable_opportunities_system,
                    super::project_territory_snapshot_system
                        .after(crate::nanobot::NanobotSimulationSet::Movement),
                )
                    .chain()
                    .in_set(RegionalAllocationSet::Project),
            )
            .add_systems(
                FixedUpdate,
                (
                    super::release_finished_regional_leases_system,
                    ApplyDeferred,
                    super::maintain_regional_leases_system,
                    ApplyDeferred,
                    advance_allocation_clock_system,
                )
                    .chain()
                    .in_set(RegionalAllocationSet::Invalidate),
            )
            .add_systems(
                FixedUpdate,
                (
                    super::reconcile_defender_responses_system,
                    ApplyDeferred,
                    super::defender_staging::reconcile_defender_staging_system,
                    regional_allocation_acquisition_system.run_if(allocation_tick_due),
                )
                    .chain()
                    .in_set(RegionalAllocationSet::Acquire),
            );
    }
}

fn advance_allocation_clock_system(
    time: Res<Time<Fixed>>,
    mut clock: ResMut<AllocationClock>,
    mut due: ResMut<AllocationTickDue>,
    mut wake: ResMut<RegionalAllocationWake>,
) {
    let elapsed = clock.advance_by(time.delta()) > 0;
    let requested = wake.take();
    due.due = !due.initialized || elapsed || requested;
    due.initialized = true;
}

fn allocation_tick_due(due: Res<AllocationTickDue>) -> bool {
    due.due
}

#[derive(Clone, Copy)]
struct BotSnapshot {
    entity: Entity,
    position: Vec2,
    region: AllocationRegion,
    swarm: SwarmId,
    kind: NanobotType,
}

#[allow(clippy::type_complexity)]
#[derive(SystemParam)]
pub struct AllocationInputs<'w, 's> {
    grid: Res<'w, crate::intent::IntentGrid>,
    facilities: Query<'w, 's, (&'static ProductionFacility, &'static Transform)>,
    chargers: Query<'w, 's, (&'static Charger, &'static Transform)>,
    defenders: Query<
        'w,
        's,
        (
            &'static Charge,
            Option<&'static Health>,
            Option<&'static ChargerAssignment>,
            Option<&'static ChargerProgress>,
        ),
    >,
    ages: ResMut<'w, TerminalDemandAges>,
    travelling: TravellingWorkers<'w, 's>,
    gather_quotas: Option<Res<'w, GatherQuotas>>,
}

/// Workers holding Maintenance or Planned Build work that have not reached it.
type TravellingWorkers<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        &'static RegionalLease,
        &'static SwarmMember,
        Option<&'static MaintenanceAssignment>,
    ),
    (
        With<Nanobot>,
        Or<(With<MaintenanceAssignment>, With<PlannedStructureClaim>)>,
        Without<MaintenanceProgress>,
        Without<PlannedStructureProgress>,
    ),
>;

/// An idle Worker takes over travelling work only when at least this many
/// times closer to the target than its current holder.
const HANDOFF_DISTANCE_RATIO: f32 = 2.0;
/// ...and when that saves at least four cells of travel, so near-equal
/// Workers do not trade the same job back and forth.
const HANDOFF_MIN_SAVING: f32 = 4.0 * crate::navigation::CELL_WIDTH;

/// Hand Maintenance and Planned Build work from a Worker still travelling to
/// it to a much nearer idle Worker. Each job holds one Worker, so without this
/// the first idle Worker anywhere keeps it while Workers beside it gather on.
#[allow(clippy::too_many_arguments)]
fn hand_off_travelling_worker_jobs(
    commands: &mut Commands,
    candidates: &mut Vec<BotSnapshot>,
    travelling: &TravellingWorkers,
    structures: &Query<&Transform>,
    planned: &mut Query<(Entity, &mut PlannedStructure, &Transform)>,
    navigation: &Navigation,
    now_tick: u64,
    lease_ttl: u64,
) {
    let mut released = std::collections::BTreeSet::new();
    let mut taken = std::collections::BTreeSet::new();
    for bot in candidates
        .iter()
        .filter(|bot| bot.kind == NanobotType::Worker)
    {
        let mut best: Option<(f32, Entity, RegionalLease, Option<MaintenanceAssignment>)> = None;
        for (holder, holder_transform, lease, member, maintenance) in travelling {
            if member.0 != bot.swarm || released.contains(&holder) {
                continue;
            }
            let (OpportunityTarget::Maintenance { structure }
            | OpportunityTarget::PlannedBuild { structure, .. }) = lease.target
            else {
                continue;
            };
            let Ok(target) = structures.get(structure) else {
                continue;
            };
            let target = target.translation.truncate();
            let own = bot.position.distance(target);
            let theirs = holder_transform.translation.truncate().distance(target);
            if own * HANDOFF_DISTANCE_RATIO > theirs || theirs - own < HANDOFF_MIN_SAVING {
                continue;
            }
            if best.as_ref().is_none_or(|(distance, ..)| own < *distance) {
                best = Some((own, holder, *lease, maintenance.copied()));
            }
        }
        let Some((_, holder, lease, maintenance)) = best else {
            continue;
        };
        let (OpportunityTarget::Maintenance { structure }
        | OpportunityTarget::PlannedBuild { structure, .. }) = lease.target
        else {
            continue;
        };
        let Ok(target_transform) = structures.get(structure) else {
            continue;
        };
        let region = InteractionRegion::structure(target_transform);
        if !matches!(
            navigation.query_connectivity(bot.position, RouteGoal::Interaction(region)),
            ConnectivityStatus::Connected { .. }
        ) {
            continue;
        }
        if let Some(assignment) = maintenance {
            commands
                .entity(bot.entity)
                .insert((assignment, region.movement_from(bot.position)));
            commands
                .entity(holder)
                .remove::<(MaintenanceAssignment, MaintenanceProgress)>();
        } else {
            let Ok((_, mut plan, transform)) = planned.get_mut(structure) else {
                continue;
            };
            if !plan.hand_over_worker(commands, structure, bot.entity, bot.position, transform) {
                continue;
            }
            commands
                .entity(holder)
                .remove::<(PlannedStructureClaim, PlannedStructureProgress)>();
        }
        commands
            .entity(holder)
            .remove::<(RegionalLease, DirectMovementComponent)>()
            .insert(Commitment::Idle);
        commands.entity(bot.entity).insert(RegionalLease::new(
            lease.region,
            lease.category,
            lease.target,
            lease.owner,
            now_tick,
            0,
            lease_ttl,
        ));
        released.insert(holder);
        taken.insert(bot.entity);
    }
    candidates.retain(|bot| !taken.contains(&bot.entity));
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn regional_allocation_acquisition_system(
    mut commands: Commands,
    clock: Res<AllocationClock>,
    tuning: Res<crate::tuning::Tuning>,
    navigation: Res<Navigation>,
    projection: Res<ActionableProjection>,
    mut region_ages: ResMut<RegionalServiceAges>,
    bots: Query<
        (
            Entity,
            &Transform,
            &NanobotType,
            &Commitment,
            &SwarmMember,
            Option<&RegionalLease>,
            Option<&crate::nanobot::work_navigation::RejectedWorkGoal>,
        ),
        With<Nanobot>,
    >,
    active_leases: Query<&RegionalLease>,
    reservations: Query<&LogisticsReservation>,
    busy: Query<
        (),
        Or<(
            With<DirectMovementComponent>,
            With<GatherAssignment>,
            With<ExtractProgress>,
            With<WorkerLoad>,
            With<ReturningToStockpile>,
            With<PlannedStructureClaim>,
            With<PlannedStructureProgress>,
            With<MaintenanceAssignment>,
            With<MaintenanceProgress>,
            With<HaulerAssignment>,
            With<HaulerLoading>,
            With<HaulerLoad>,
        )>,
    >,
    charge_busy: Query<(), Or<(With<ChargerAssignment>, With<ChargerProgress>)>>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    mut planned: Query<(Entity, &mut PlannedStructure, &Transform)>,
    structures: Query<&Transform>,
    stockpiles: Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
    mut terminal: AllocationInputs,
) {
    if projection
        .iter_regions()
        .all(|(_, opportunities)| opportunities.is_empty())
    {
        region_ages.waiting.clear();
        terminal.ages.waiting.clear();
        return;
    }

    let facilities = &terminal.facilities;
    let chargers = &terminal.chargers;

    let mut claim_counts = BTreeMap::new();
    for lease in &active_leases {
        *claim_counts
            .entry(claim_key(lease.target, lease.owner))
            .or_insert(0) += 1;
    }
    let mut reserved_source = BTreeMap::<Entity, u32>::new();
    let mut reserved_destination = BTreeMap::<Entity, u32>::new();
    for reservation in &reservations {
        *reserved_source.entry(reservation.source).or_default() += reservation.source_remaining;
        *reserved_destination
            .entry(reservation.destination)
            .or_default() += reservation.destination_remaining;
    }

    let mut charger_demand = BTreeMap::<Entity, (u8, u32)>::new();
    for (charge, health, assignment, progress) in &terminal.defenders {
        let charger = progress
            .map(|progress| progress.charger)
            .or_else(|| assignment.map(|assignment| assignment.charger));
        let Some(charger) = charger else {
            continue;
        };
        let urgency =
            if charge.is_empty() || health.is_some_and(|health| health.current < health.max) {
                0
            } else if charge.current < tuning.charge.weakened_threshold {
                1
            } else if charge.current <= tuning.charge.low_threshold {
                2
            } else {
                4
            };
        let entry = charger_demand.entry(charger).or_insert((urgency, 0));
        entry.0 = entry.0.min(urgency);
        entry.1 = entry.1.saturating_add(minerals_to_fully_charge(
            charge.current,
            charge.max,
            &tuning.charge,
        ));
    }

    let mut active_terminals = BTreeMap::new();
    let mut nearest_terminal_source = BTreeMap::<Entity, f32>::new();
    for (_, opportunities) in projection.iter_regions() {
        for opportunity in opportunities {
            let OpportunityTarget::Haul { source, sink, .. } = opportunity.target else {
                continue;
            };
            let Some(terminal_transform) = facilities
                .get(sink)
                .map(|(_, transform)| transform)
                .or_else(|_| chargers.get(sink).map(|(_, transform)| transform))
                .ok()
            else {
                continue;
            };
            active_terminals.insert(sink, ());
            let Ok((stockpile, source_transform, _)) = stockpiles.get(source) else {
                continue;
            };
            let reserved = reserved_source.get(&source).copied().unwrap_or_default();
            if stockpile.amount <= reserved {
                continue;
            }
            let distance = source_transform
                .translation
                .truncate()
                .distance(terminal_transform.translation.truncate());
            let nearest = nearest_terminal_source.entry(sink).or_insert(f32::INFINITY);
            *nearest = nearest.min(distance);
        }
    }
    terminal
        .ages
        .waiting
        .retain(|terminal, _| active_terminals.contains_key(terminal));
    for terminal_entity in active_terminals.keys().copied() {
        let age = terminal.ages.waiting.entry(terminal_entity).or_default();
        *age = age.saturating_add(1);
    }

    let planned_workers = planned
        .iter_mut()
        .map(|(entity, planned, _)| {
            (
                entity.to_bits(),
                planned.active_worker().map(|worker| worker.to_bits()),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut candidates = bots
        .iter()
        .filter_map(|(entity, transform, kind, commitment, swarm, lease, _)| {
            if *commitment != Commitment::Idle {
                return None;
            }
            if lease.is_some() {
                return None;
            }
            if busy.contains(entity) || charge_busy.contains(entity) {
                return None;
            }
            Some(BotSnapshot {
                entity,
                position: transform.translation.truncate(),
                region: AllocationRegion::for_cell(crate::nanobot::world_to_cell(
                    transform.translation.truncate(),
                )),
                swarm: swarm.0,
                kind: *kind,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|bot| bot.entity.to_bits());
    hand_off_travelling_worker_jobs(
        &mut commands,
        &mut candidates,
        &terminal.travelling,
        &structures,
        &mut planned,
        &navigation,
        clock.tick(),
        tuning.autonomy.lease_no_progress_ttl_ticks,
    );

    let work_by_kind = NanobotType::ALL.map(|kind| {
        projection
            .iter_regions()
            .filter_map(|(region, opportunities)| {
                let eligible = opportunities
                    .iter()
                    .copied()
                    .filter(|work| kind_allows(kind, work.category))
                    .collect::<Vec<_>>();
                (!eligible.is_empty()).then_some((region, eligible))
            })
            .collect::<Vec<_>>()
    });
    let candidate_swarms = candidates
        .iter()
        .map(|bot| bot.swarm)
        .collect::<std::collections::BTreeSet<_>>();
    // Gather slots left under each swarm's Gather Quota. A swarm without a
    // quota (no Population Demand in this app) gathers without a cap.
    let mut gather_open = terminal
        .gather_quotas
        .as_deref()
        .map(|quotas| {
            quotas
                .iter()
                .map(|(swarm, economy)| {
                    (
                        swarm,
                        economy
                            .gather_quota
                            .saturating_sub(economy.gathering_workers),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut work_by_swarm_kind = BTreeMap::new();
    for swarm in candidate_swarms.iter().copied() {
        for (kind, regional_work) in work_by_kind.iter().enumerate() {
            let regions = regional_work
                .iter()
                .filter_map(|(region, opportunities)| {
                    let compatible = opportunities
                        .iter()
                        .copied()
                        .filter(|work| work.owner.is_none_or(|owner| owner == swarm))
                        .filter(|work| !gather_closed(&gather_open, swarm, *work))
                        .collect::<Vec<_>>();
                    (!compatible.is_empty()).then_some((*region, compatible))
                })
                .collect::<Vec<_>>();
            work_by_swarm_kind.insert((swarm, kind), regions);
        }
    }
    let active_region_kinds = work_by_swarm_kind
        .iter()
        .flat_map(|((swarm, kind), regions)| {
            regions
                .iter()
                .map(move |(region, _)| (*swarm, *region, *kind))
        })
        .collect::<std::collections::BTreeSet<_>>();
    region_ages
        .waiting
        .retain(|key, _| active_region_kinds.contains(key));
    for key in active_region_kinds {
        let age = region_ages.waiting.entry(key).or_default();
        *age = age.saturating_add(1);
    }

    let mut capacities = BTreeMap::<(SwarmId, AllocationRegion, usize), u32>::new();
    let mut source_regions = std::collections::BTreeSet::new();
    for bot in &candidates {
        source_regions.insert((bot.swarm, bot.region));
        let capacity = capacities
            .entry((bot.swarm, bot.region, kind_index(bot.kind)))
            .or_default();
        *capacity = capacity.saturating_add(1);
    }

    let mut pulls = BTreeMap::new();
    let mut ordered_regions = BTreeMap::new();
    for ((swarm, kind), regional_work) in &work_by_swarm_kind {
        let region_slices = regional_work
            .iter()
            .map(|(region, work)| (*region, work.as_slice()))
            .collect::<Vec<_>>();
        let unclaimed_regions = regional_work
            .iter()
            .map(|(region, work)| {
                let unclaimed = work
                    .iter()
                    .map(|work| {
                        let claims = claims_for(&claim_counts, *work);
                        let available_work = if gather_ring_full(*work, claims, &deposits) {
                            0
                        } else {
                            gather_quota_work(
                                &gather_open,
                                *swarm,
                                *work,
                                unclaimed_work(*work, claims, &tuning),
                                &tuning,
                            )
                        };
                        ActionableOpportunity {
                            available_work,
                            ..*work
                        }
                    })
                    .collect::<Vec<_>>();
                (*region, unclaimed)
            })
            .collect::<Vec<_>>();
        let pressures = pressure_map(
            unclaimed_regions
                .iter()
                .map(|(region, work)| (*region, work.as_slice())),
            CategoryWeights::new([
                tuning.autonomy.gather_weight,
                tuning.autonomy.planned_build_weight,
                tuning.autonomy.maintenance_weight,
                tuning.autonomy.haul_weight,
            ]),
        )
        .into_values()
        .collect::<Vec<_>>();
        let kind_capacities = capacities
            .iter()
            .filter_map(|((candidate_swarm, region, candidate_kind), capacity)| {
                (*candidate_swarm == *swarm && *candidate_kind == *kind)
                    .then_some((*region, *capacity))
            })
            .collect::<Vec<_>>();
        for (source, pull) in outward_pull_budgets(&kind_capacities, &pressures, u32::MAX) {
            pulls.insert((*swarm, source, *kind), pull);
        }
        for (source_swarm, source) in source_regions.iter().copied() {
            if source_swarm != *swarm {
                continue;
            }
            let mut ordered = region_slices.clone();
            ordered.sort_by_key(|(region, _)| {
                region_fairness_sort_key(
                    source,
                    *region,
                    region_ages
                        .waiting
                        .get(&(*swarm, *region, *kind))
                        .copied()
                        .unwrap_or_default(),
                    tuning.autonomy.regional_fairness_promotion_ticks,
                )
            });
            ordered_regions.insert((*swarm, source, *kind), ordered);
        }
    }
    let bounds = CandidateBounds {
        max_regions: RUNTIME_MAX_CANDIDATE_REGIONS,
        max_candidates: RUNTIME_MAX_CANDIDATES,
    };

    for bot in candidates {
        let bot_key = (bot.swarm, bot.region, kind_index(bot.kind));
        let Some(pull) = pulls.get(&bot_key).copied() else {
            continue;
        };
        let Some(ordered) = ordered_regions.get(&bot_key) else {
            continue;
        };
        let rejected = bots
            .get(bot.entity)
            .ok()
            .and_then(|(_, _, _, _, _, _, rejected)| rejected);
        let work = if bot.kind == NanobotType::Hauler {
            let Some(selection) = choose_terminal_logistics_work(
                &tuning,
                bot,
                pull,
                ordered,
                bounds,
                rejected.map(|rejected| rejected.region),
                &stockpiles,
                facilities,
                chargers,
                &navigation,
                &reserved_source,
                &reserved_destination,
                &charger_demand,
                &terminal.ages,
                &nearest_terminal_source,
            ) else {
                continue;
            };
            let reservation = selection.reservation;
            commands.entity(bot.entity).insert((
                HaulerAssignment {
                    source: reservation.source,
                    sink: reservation.destination,
                    pickup_role: selection.pickup_role,
                },
                reservation,
                selection.pickup.movement_from(bot.position),
            ));
            *reserved_source.entry(reservation.source).or_default() += reservation.source_remaining;
            *reserved_destination
                .entry(reservation.destination)
                .or_default() += reservation.destination_remaining;
            selection.work
        } else {
            let mut gather_approaches = BTreeMap::new();
            let Some(decision) = choose_bounded_candidate_from_ordered_regions_with_claims(
                allocation_candidate(bot),
                pull,
                ordered.iter().copied(),
                bounds,
                |work| {
                    if gather_closed(&gather_open, bot.swarm, work) {
                        return None;
                    }
                    let claims = claims_for(&claim_counts, work);
                    if !target_available(
                        &tuning,
                        bot,
                        work,
                        claims,
                        &planned_workers,
                        &deposits,
                        &structures,
                        &stockpiles,
                    ) {
                        return None;
                    }
                    let reachable = |region| {
                        rejected.is_none_or(|rejected| rejected.region != region)
                            && matches!(
                                navigation.query_connectivity(
                                    bot.position,
                                    RouteGoal::Interaction(region)
                                ),
                                ConnectivityStatus::Connected { .. }
                            )
                    };
                    let region = match work.target {
                        OpportunityTarget::Gather { deposit, cell } => {
                            let (state, transform) = deposits.get(deposit).ok()?;
                            let preferred = InteractionRegion::deposit(transform, state.radius)
                                .within_cell(cell);
                            let approach = if reachable(preferred) {
                                (cell, preferred)
                            } else {
                                painted_gather_regions(
                                    &terminal.grid,
                                    bot.swarm,
                                    *transform,
                                    state.radius,
                                )
                                .filter(|(candidate, region)| {
                                    *candidate != cell && reachable(*region)
                                })
                                .min_by_key(|(candidate, _)| (candidate.x, candidate.y))?
                            };
                            gather_approaches.insert(deposit, approach);
                            return Some(claims);
                        }
                        OpportunityTarget::PlannedBuild { structure, .. }
                        | OpportunityTarget::Maintenance { structure } => structures
                            .get(structure)
                            .ok()
                            .map(InteractionRegion::structure),
                        OpportunityTarget::Haul { .. } => None,
                    }?;
                    reachable(region).then_some(claims)
                },
            ) else {
                continue;
            };
            if !assign_worker_work(
                &mut commands,
                bot,
                decision.opportunity,
                &gather_approaches,
                &mut planned,
                &structures,
            ) {
                continue;
            }
            decision.opportunity
        };
        if let OpportunityTarget::Haul { sink, .. } = work.target
            && (facilities.get(sink).is_ok() || chargers.get(sink).is_ok())
        {
            terminal.ages.waiting.insert(sink, 0);
        }
        region_ages
            .waiting
            .insert((bot.swarm, work.region, kind_index(bot.kind)), 0);
        let lease_owner = work.owner;
        let lease = RegionalLease::new(
            work.region,
            work.category,
            work.target,
            lease_owner,
            clock.tick(),
            0,
            tuning.autonomy.lease_no_progress_ttl_ticks,
        );
        commands.entity(bot.entity).insert(lease);
        *claim_counts
            .entry(claim_key(work.target, lease_owner))
            .or_insert(0) += 1;
        if let Some(pull) = pulls.get_mut(&bot_key) {
            let remaining = pull.categories.get(work.category).saturating_sub(1);
            pull.categories.set(work.category, remaining);
        }
        if work.category == OpportunityCategory::Gather
            && let Some(open) = gather_open.get_mut(&bot.swarm)
        {
            *open = open.saturating_sub(1);
        }
    }
}

/// Base urgency of a startable production cycle. Lower base urgencies are
/// Charger emergencies that sustain committed Defenders.
const STARTABLE_PRODUCTION_URGENCY: u8 = 3;

/// How much farther than a terminal's nearest stocked Sink Stockpile another
/// pickup may be: two cells, so near-equal Sinks share the load.
const NEAREST_TERMINAL_SOURCE_SLACK: f32 = 2.0 * crate::navigation::CELL_WIDTH;

/// Strict tier: urgency, waiting-age fairness, then proportional deficit.
type StrictRank = (u8, u32, u64);

#[derive(Clone, Copy)]
struct TerminalLogisticsScore {
    /// `Some` for Charger emergencies and fully aged terminals, which keep
    /// strict ordering ahead of all other haul work.
    strict: Option<StrictRank>,
    /// Route cost per mineral moved plus a penalty per urgency level, so a
    /// long trip carrying a few minerals loses to a short full refill.
    weighted_cost: f32,
    estimated_cost: f32,
    terminal: u64,
    source: u64,
}

impl TerminalLogisticsScore {
    fn cmp(self, other: Self) -> Ordering {
        let rank = match (self.strict, other.strict) {
            (Some(own), Some(theirs)) => own
                .cmp(&theirs)
                .then_with(|| self.estimated_cost.total_cmp(&other.estimated_cost)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => self.weighted_cost.total_cmp(&other.weighted_cost),
        };
        rank.then_with(|| self.terminal.cmp(&other.terminal))
            .then_with(|| self.source.cmp(&other.source))
    }
}

struct SelectedHaul {
    work: ActionableOpportunity,
    pickup: InteractionRegion,
    pickup_role: StockpileRole,
    reservation: LogisticsReservation,
}

#[allow(clippy::too_many_arguments)]
fn choose_terminal_logistics_work(
    tuning: &crate::tuning::Tuning,
    bot: BotSnapshot,
    pull: super::RegionalPullBudget,
    ordered: &[(AllocationRegion, &[ActionableOpportunity])],
    bounds: CandidateBounds,
    rejected: Option<InteractionRegion>,
    stockpiles: &Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
    facilities: &Query<(&ProductionFacility, &Transform)>,
    chargers: &Query<(&Charger, &Transform)>,
    navigation: &Navigation,
    reserved_source: &BTreeMap<Entity, u32>,
    reserved_destination: &BTreeMap<Entity, u32>,
    charger_demand: &BTreeMap<Entity, (u8, u32)>,
    ages: &TerminalDemandAges,
    nearest_terminal_source: &BTreeMap<Entity, f32>,
) -> Option<SelectedHaul> {
    if pull.categories.get(OpportunityCategory::Haul) == 0 {
        return None;
    }
    let mut examined = 0;
    let mut best: Option<(TerminalLogisticsScore, SelectedHaul)> = None;
    for (_, opportunities) in ordered.iter().take(bounds.max_regions) {
        for work in *opportunities {
            if examined == bounds.max_candidates {
                break;
            }
            examined += 1;
            let OpportunityTarget::Haul { source, sink, kind } = work.target else {
                continue;
            };
            if work.owner.is_some_and(|owner| owner != bot.swarm) {
                continue;
            }
            let Ok((source_state, source_transform, source_role)) = stockpiles.get(source) else {
                continue;
            };
            let pickup = InteractionRegion::structure(source_transform);
            if rejected == Some(pickup) {
                continue;
            }
            let (buffer, sink_transform) = if let Ok((facility, transform)) = facilities.get(sink) {
                (
                    PickupBuffer {
                        role: DestinationRole::ProductionFacility,
                        buffered: facility.input_amount,
                        capacity: facility.claimable_capacity(tuning),
                    },
                    transform,
                )
            } else if let Ok((charger, transform)) = chargers.get(sink) {
                (
                    PickupBuffer {
                        role: DestinationRole::Charger,
                        buffered: charger.amount,
                        capacity: charger.capacity,
                    },
                    transform,
                )
            } else if let Ok((stockpile, transform, _)) = stockpiles.get(sink) {
                (
                    PickupBuffer {
                        role: DestinationRole::Stockpile(StockpileRole::Sink),
                        buffered: stockpile.amount,
                        capacity: stockpile.capacity,
                    },
                    transform,
                )
            } else {
                continue;
            };
            // A terminal draws from its nearest stocked Sink Stockpiles. A far
            // pickup would hold the terminal's claim for the whole long trip
            // while haulers beside a nearer stocked Sink find nothing to claim.
            if let Some(nearest) = nearest_terminal_source.get(&sink) {
                let delivery = source_transform
                    .translation
                    .truncate()
                    .distance(sink_transform.translation.truncate());
                if delivery > nearest + NEAREST_TERMINAL_SOURCE_SLACK {
                    continue;
                }
            }
            let quote = buffer.quote(
                source_state.amount,
                reserved_source.get(&source).copied().unwrap_or_default(),
                reserved_destination.get(&sink).copied().unwrap_or_default(),
                charger_demand.get(&sink).copied(),
                tuning.logistics.hauler_carry_capacity,
                tuning.production.cost_per_bot,
            );
            if quote.amount == 0 {
                continue;
            }
            let age = ages.waiting_ticks(sink);
            let urgency = u32::from(quote.base_urgency)
                .saturating_sub(age / tuning.autonomy.terminal_fairness_promotion_ticks.max(1))
                as u8;
            let deficit_ratio = u64::from(quote.unclaimed_space).saturating_mul(1_000_000)
                / u64::from(buffer.capacity.max(1));
            let source_pos =
                match navigation.query_connectivity(bot.position, RouteGoal::Interaction(pickup)) {
                    ConnectivityStatus::Connected { endpoint } => endpoint,
                    ConnectivityStatus::Pending | ConnectivityStatus::Unreachable => continue,
                };
            let sink_pos = match navigation.query_connectivity(
                source_pos,
                RouteGoal::Interaction(InteractionRegion::structure(sink_transform)),
            ) {
                ConnectivityStatus::Connected { endpoint } => endpoint,
                ConnectivityStatus::Pending | ConnectivityStatus::Unreachable => continue,
            };
            let estimated_cost = bot.position.distance(source_pos) + source_pos.distance(sink_pos);
            if !estimated_cost.is_finite() {
                continue;
            }
            let strict = (quote.base_urgency < STARTABLE_PRODUCTION_URGENCY || urgency == 0)
                .then_some((urgency, u32::MAX - age, u64::MAX - deficit_ratio));
            let score = TerminalLogisticsScore {
                strict,
                weighted_cost: estimated_cost / quote.amount as f32
                    + f32::from(urgency) * tuning.logistics.haul_urgency_cost,
                estimated_cost,
                terminal: sink.to_bits(),
                source: source.to_bits(),
            };
            if best
                .as_ref()
                .is_none_or(|(current, _)| score.cmp(*current).is_lt())
            {
                best = Some((
                    score,
                    SelectedHaul {
                        work: *work,
                        pickup,
                        pickup_role: source_role.copied().unwrap_or_default(),
                        reservation: LogisticsReservation::new(source, sink, kind, quote.amount),
                    },
                ));
            }
        }
    }
    best.map(|(_, work)| work)
}

fn kind_index(kind: NanobotType) -> usize {
    match kind {
        NanobotType::Worker => 0,
        NanobotType::Hauler => 1,
        NanobotType::Defender => 2,
    }
}

fn kind_allows(kind: NanobotType, category: OpportunityCategory) -> bool {
    matches!(
        (kind, category),
        (NanobotType::Worker, OpportunityCategory::Gather)
            | (NanobotType::Worker, OpportunityCategory::PlannedBuild)
            | (NanobotType::Worker, OpportunityCategory::Maintenance)
            | (NanobotType::Hauler, OpportunityCategory::Haul)
    )
}

fn allocation_candidate(bot: BotSnapshot) -> AllocationCandidate {
    let eligibility = match bot.kind {
        NanobotType::Worker => CategoryEligibility::worker(),
        NanobotType::Defender => CategoryEligibility::none(),
        NanobotType::Hauler => CategoryEligibility::only(OpportunityCategory::Haul),
    };
    AllocationCandidate {
        entity_bits: bot.entity.to_bits(),
        region: bot.region,
        owner: Some(bot.swarm),
        eligibility,
    }
}

#[allow(clippy::too_many_arguments)]
fn target_available(
    tuning: &crate::tuning::Tuning,
    bot: BotSnapshot,
    work: ActionableOpportunity,
    claims: usize,
    planned_workers: &BTreeMap<u64, Option<u64>>,
    deposits: &Query<(&ResourceDeposit, &Transform)>,
    structures: &Query<&Transform>,
    stockpiles: &Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
) -> bool {
    if claims >= opportunity_capacity(work, tuning) {
        return false;
    }
    match work.target {
        OpportunityTarget::Gather { .. } => !gather_ring_full(work, claims, deposits),
        OpportunityTarget::PlannedBuild { structure, .. } => planned_workers
            .get(&structure.to_bits())
            .is_some_and(|worker| worker.is_none() || *worker == Some(bot.entity.to_bits())),
        OpportunityTarget::Maintenance { structure } => structures.get(structure).is_ok(),
        OpportunityTarget::Haul { source, .. } => stockpiles
            .get(source)
            .is_ok_and(|(stockpile, _, _)| stockpile.amount > 0 && work.available_work > 0),
    }
}

/// Gather work once the swarm's gatherers have reached its Gather Quota.
fn gather_closed(
    gather_open: &BTreeMap<SwarmId, u32>,
    swarm: SwarmId,
    work: ActionableOpportunity,
) -> bool {
    work.category == OpportunityCategory::Gather && gather_open.get(&swarm) == Some(&0)
}

/// Gather pressure limited to the loads the open quota slots can carry. Raw
/// deposit minerals would hand Gather nearly every pull slot, and Workers
/// refused at the quota would then find no budget left for other work.
fn gather_quota_work(
    gather_open: &BTreeMap<SwarmId, u32>,
    swarm: SwarmId,
    work: ActionableOpportunity,
    available_work: u32,
    tuning: &crate::tuning::Tuning,
) -> u32 {
    match gather_open.get(&swarm) {
        Some(open) if work.category == OpportunityCategory::Gather => {
            available_work.min(open.saturating_mul(tuning.logistics.worker_carry_capacity.max(1)))
        }
        _ => available_work,
    }
}

/// A deposit takes no more gatherers than fit around its work ring; a missing
/// deposit has no room at all.
fn gather_ring_full(
    work: ActionableOpportunity,
    claims: usize,
    deposits: &Query<(&ResourceDeposit, &Transform)>,
) -> bool {
    let OpportunityTarget::Gather { deposit, .. } = work.target else {
        return false;
    };
    deposits
        .get(deposit)
        .ok()
        .is_none_or(|(state, _)| claims >= gather_worker_slots(state.radius) as usize)
}

/// Work one claimant takes on, or `None` when a single claimant takes it all.
fn claim_unit(category: OpportunityCategory, tuning: &crate::tuning::Tuning) -> Option<u32> {
    match category {
        OpportunityCategory::Gather => Some(tuning.logistics.worker_carry_capacity.max(1)),
        OpportunityCategory::Haul => Some(tuning.logistics.hauler_carry_capacity.max(1)),
        OpportunityCategory::PlannedBuild | OpportunityCategory::Maintenance => None,
    }
}

fn opportunity_capacity(work: ActionableOpportunity, tuning: &crate::tuning::Tuning) -> usize {
    claim_unit(work.category, tuning)
        .map_or(1, |unit| work.available_work.div_ceil(unit))
        .max(1) as usize
}

/// Work still open to another claimant. Fully claimed opportunities exert no
/// pull, so they cannot reserve an idle bot's only category slot.
fn unclaimed_work(
    work: ActionableOpportunity,
    claims: usize,
    tuning: &crate::tuning::Tuning,
) -> u32 {
    if claims >= opportunity_capacity(work, tuning) {
        return 0;
    }
    let Some(unit) = claim_unit(work.category, tuning) else {
        return work.available_work;
    };
    let claimed = u32::try_from(claims)
        .unwrap_or(u32::MAX)
        .saturating_mul(unit);
    work.available_work.saturating_sub(claimed)
}

fn assign_worker_work(
    commands: &mut Commands,
    bot: BotSnapshot,
    work: ActionableOpportunity,
    gather_approaches: &BTreeMap<Entity, (IVec2, InteractionRegion)>,
    planned: &mut Query<(Entity, &mut PlannedStructure, &Transform)>,
    structures: &Query<&Transform>,
) -> bool {
    match work.target {
        OpportunityTarget::Gather { deposit, .. } => {
            let Some(&(cell, approach)) = gather_approaches.get(&deposit) else {
                return false;
            };
            commands.entity(bot.entity).insert((
                GatherAssignment::from_paint(cell, deposit),
                approach.movement_from(bot.position),
            ));
        }
        OpportunityTarget::PlannedBuild { structure, .. } => {
            let Ok((_, mut planned_state, transform)) = planned.get_mut(structure) else {
                return false;
            };
            if !planned_state.assign_worker(
                commands,
                structure,
                bot.entity,
                bot.position,
                transform,
            ) {
                return false;
            }
        }
        OpportunityTarget::Maintenance { structure } => {
            let Ok(transform) = structures.get(structure) else {
                return false;
            };
            commands.entity(bot.entity).insert((
                MaintenanceAssignment {
                    cell: work.cell,
                    target: structure,
                },
                InteractionRegion::structure(transform).movement_from(bot.position),
            ));
        }
        OpportunityTarget::Haul { .. } => return false,
    }
    true
}

fn region_distance_key(left: AllocationRegion, right: AllocationRegion) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y)
}

fn claims_for(
    claim_counts: &BTreeMap<(u8, u64, u64, u64, Option<SwarmId>), usize>,
    work: ActionableOpportunity,
) -> usize {
    claim_counts
        .get(&claim_key(work.target, work.owner))
        .copied()
        .unwrap_or(0)
}

fn claim_key(
    target: OpportunityTarget,
    owner: Option<SwarmId>,
) -> (u8, u64, u64, u64, Option<SwarmId>) {
    match target {
        OpportunityTarget::Gather { deposit, .. } => (0, deposit.to_bits(), 0, 0, owner),
        OpportunityTarget::PlannedBuild { structure, .. } => (1, structure.to_bits(), 0, 0, owner),
        OpportunityTarget::Maintenance { structure } => (2, structure.to_bits(), 0, 0, owner),
        OpportunityTarget::Haul { source, sink, .. } => {
            (3, source.to_bits(), sink.to_bits(), 0, owner)
        }
    }
}

/// Reconcile allocation policy without discarding acquired work or fairness debt.
pub fn reconcile_autonomy_tuning(
    world: &mut World,
    old: &crate::tuning::Tuning,
    new: &crate::tuning::Tuning,
) {
    if old.autonomy == new.autonomy
        && old.construction == new.construction
        && old.logistics == new.logistics
        && old.charge == new.charge
        && old.production == new.production
        && old.maintenance == new.maintenance
    {
        return;
    }
    if old.autonomy.lease_no_progress_ttl_ticks != new.autonomy.lease_no_progress_ttl_ticks {
        let now = world
            .get_resource::<AllocationClock>()
            .map_or(0, AllocationClock::tick);
        for mut lease in world.query::<&mut RegionalLease>().iter_mut(world) {
            lease.rescale_deadline(
                now,
                old.autonomy.lease_no_progress_ttl_ticks,
                new.autonomy.lease_no_progress_ttl_ticks,
            );
        }
    }
    let active_cells: Vec<_> = world
        .get_resource::<crate::intent::IntentGrid>()
        .map(|grid| grid.iter_active_cells().map(|(cell, _)| cell).collect())
        .unwrap_or_default();
    if let Some(mut projection) = world.get_resource_mut::<ActionableProjection>() {
        let regions: Vec<_> = projection
            .iter_regions()
            .map(|(region, _)| region)
            .collect();
        for region in regions {
            projection.invalidate_region(region);
        }
        for cell in active_cells {
            projection.invalidate_cell(cell);
        }
    }
    if let Some(mut wake) = world.get_resource_mut::<RegionalAllocationWake>() {
        wake.request_current_pass();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_claim_key_is_shared_by_deposit_across_anchors() {
        let deposit = Entity::PLACEHOLDER;
        let first = OpportunityTarget::Gather {
            deposit,
            cell: IVec2::ZERO,
        };
        let second = OpportunityTarget::Gather {
            deposit,
            cell: IVec2::new(8, 3),
        };

        assert_eq!(claim_key(first, None), claim_key(second, None));
    }

    #[test]
    fn claimed_work_leaves_only_the_unclaimed_share_as_pressure() {
        let mut tuning = crate::tuning::Tuning::default();
        tuning.logistics.worker_carry_capacity = 4;
        let gather = ActionableOpportunity {
            region: AllocationRegion { x: 0, y: 0 },
            category: OpportunityCategory::Gather,
            target: OpportunityTarget::Gather {
                deposit: Entity::PLACEHOLDER,
                cell: IVec2::ZERO,
            },
            cell: IVec2::ZERO,
            owner: None,
            available_work: 10,
        };
        let maintenance = ActionableOpportunity {
            category: OpportunityCategory::Maintenance,
            target: OpportunityTarget::Maintenance {
                structure: Entity::PLACEHOLDER,
            },
            available_work: 1,
            ..gather
        };

        assert_eq!(unclaimed_work(gather, 0, &tuning), 10);
        assert_eq!(unclaimed_work(gather, 2, &tuning), 2);
        assert_eq!(unclaimed_work(gather, 3, &tuning), 0);
        assert_eq!(unclaimed_work(maintenance, 0, &tuning), 1);
        assert_eq!(unclaimed_work(maintenance, 1, &tuning), 0);

        tuning.logistics.hauler_carry_capacity = 10;
        let haul = ActionableOpportunity {
            category: OpportunityCategory::Haul,
            target: OpportunityTarget::Haul {
                source: Entity::PLACEHOLDER,
                sink: Entity::PLACEHOLDER,
                kind: crate::resources::ResourceKind::Minerals,
            },
            available_work: 25,
            ..gather
        };
        assert_eq!(unclaimed_work(haul, 2, &tuning), 5);
        assert_eq!(unclaimed_work(haul, 3, &tuning), 0);
    }
}
