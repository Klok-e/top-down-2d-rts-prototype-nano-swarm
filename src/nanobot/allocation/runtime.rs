//! Production ECS adapter for deterministic regional allocation.

use std::{cmp::Ordering, collections::BTreeMap};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{
    ActionableOpportunity, ActionableProjection, AllocationCandidate, AllocationClock,
    AllocationRegion, CandidateBounds, CategoryEligibility, CategoryWeights, OpportunityCategory,
    OpportunityTarget, RegionalLease, RegionalLeaseConfig, RegionalLeaseState,
    choose_bounded_candidate_from_ordered_regions_with_claims, outward_pull_budgets, pressure_map,
};
use crate::{
    ZONE_BLOCK_SIZE,
    intent::IntentGrid,
    nanobot::{
        BUILDING_FOOTPRINT_RADIUS, Commitment, DEFEND_IN_CELL_STOP_RADIUS, DefendAssignment,
        DefendHold, DirectMovementComponent, ExtractProgress, GatherAssignment,
        HAULER_CARRY_CAPACITY, HaulerAssignment, HaulerLoad, HaulerLoading, Health,
        LogisticsReservation, MaintenanceAssignment, MaintenanceProgress, Nanobot, NanobotType,
        PRODUCTION_COST_PER_BOT, PlannedStructure, PlannedStructureClaim, PlannedStructureProgress,
        ProductionFacility, ReturningToStockpile, SwarmId, SwarmMember, WORKER_CARRY_CAPACITY,
        WorkerLoad,
        charge::{
            Charge, Charger, ChargerAssignment, ChargerProgress, LOW_CHARGE_THRESHOLD,
            WEAKENED_CHARGE_THRESHOLD, minerals_to_fully_charge,
        },
        hauler_route_cost, planned_route_movement,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
};

/// Maximum projection buckets examined for one nanobot acquisition.
pub const RUNTIME_MAX_CANDIDATE_REGIONS: usize = 16;
/// Maximum exact opportunities examined for one nanobot acquisition.
pub const RUNTIME_MAX_CANDIDATES: usize = 128;

/// Allocation ticks waited before fairness promotes terminal urgency one tier.
pub const TERMINAL_FAIRNESS_PROMOTION_TICKS: u32 = 8;

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
struct AllocationTickDue {
    due: bool,
    initialized: bool,
}

impl Default for AllocationTickDue {
    fn default() -> Self {
        Self {
            due: true,
            initialized: false,
        }
    }
}

/// Single production allocator. Projection and invalidation run every update;
/// new claims run on deterministic 100 ms simulation-time boundaries.
pub struct RegionalAllocationPlugin;

impl Plugin for RegionalAllocationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionableProjection>()
            .init_resource::<AllocationClock>()
            .init_resource::<RegionalLeaseConfig>()
            .init_resource::<AllocationTickDue>()
            .init_resource::<TerminalDemandAges>()
            .configure_sets(
                Update,
                (
                    RegionalAllocationSet::Project,
                    RegionalAllocationSet::Invalidate,
                    RegionalAllocationSet::Acquire,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                super::project_actionable_opportunities_system
                    .in_set(RegionalAllocationSet::Project),
            )
            .add_systems(
                Update,
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
                Update,
                regional_allocation_acquisition_system
                    .run_if(allocation_tick_due)
                    .in_set(RegionalAllocationSet::Acquire),
            );
    }
}

fn advance_allocation_clock_system(
    time: Res<Time>,
    mut clock: ResMut<AllocationClock>,
    mut due: ResMut<AllocationTickDue>,
) {
    let elapsed = clock.advance_by(time.delta()) > 0;
    due.due = !due.initialized || elapsed;
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
    resume_pending: bool,
}

#[allow(clippy::type_complexity)]
#[derive(SystemParam)]
pub struct TerminalLogisticsParams<'w, 's> {
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
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn regional_allocation_acquisition_system(
    mut commands: Commands,
    clock: Res<AllocationClock>,
    grid: Res<IntentGrid>,
    projection: Res<ActionableProjection>,
    bots: Query<
        (
            Entity,
            &Transform,
            &NanobotType,
            &Commitment,
            &SwarmMember,
            Option<&RegionalLease>,
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
            With<DefendAssignment>,
            With<DefendHold>,
            With<HaulerAssignment>,
            With<HaulerLoading>,
            With<HaulerLoad>,
        )>,
    >,
    charge_busy: Query<(), Or<(With<ChargerAssignment>, With<ChargerProgress>)>>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    mut planned: Query<(Entity, &mut PlannedStructure, &Transform)>,
    structures: Query<&Transform>,
    stockpiles: Query<(&Stockpile, &Transform)>,
    mut terminal: TerminalLogisticsParams,
) {
    let facilities = &terminal.facilities;
    let chargers = &terminal.chargers;

    let mut claim_counts = BTreeMap::new();
    for lease in active_leases
        .iter()
        .filter(|lease| lease.counts_toward_capacity())
    {
        *claim_counts.entry(claim_key(lease.target)).or_insert(0) += 1;
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
            } else if charge.current < WEAKENED_CHARGE_THRESHOLD {
                1
            } else if charge.current <= LOW_CHARGE_THRESHOLD {
                2
            } else {
                4
            };
        let entry = charger_demand.entry(charger).or_insert((urgency, 0));
        entry.0 = entry.0.min(urgency);
        entry.1 = entry
            .1
            .saturating_add(minerals_to_fully_charge(charge.current, charge.max));
    }

    let mut active_terminals = BTreeMap::new();
    for (_, opportunities) in projection.iter_regions() {
        for opportunity in opportunities {
            let OpportunityTarget::Haul { sink, .. } = opportunity.target else {
                continue;
            };
            if facilities.get(sink).is_ok() || chargers.get(sink).is_ok() {
                active_terminals.insert(sink, ());
            }
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
                planned.active_worker.map(|worker| worker.to_bits()),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut candidates = bots
        .iter()
        .filter_map(|(entity, transform, kind, commitment, swarm, lease)| {
            if *commitment != Commitment::Idle {
                return None;
            }
            let resume_pending =
                lease.is_some_and(|lease| lease.state == RegionalLeaseState::ResumePending);
            if lease.is_some() && !resume_pending {
                return None;
            }
            if lease.is_none() && (busy.contains(entity) || charge_busy.contains(entity)) {
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
                resume_pending,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|bot| bot.entity.to_bits());

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
    let mut capacities = BTreeMap::<(AllocationRegion, usize), u32>::new();
    let mut source_regions = BTreeMap::new();
    for bot in &candidates {
        let source = bot.region;
        source_regions.insert(source, ());
        let capacity = capacities
            .entry((source, kind_index(bot.kind)))
            .or_default();
        *capacity = capacity.saturating_add(1);
    }

    let mut pulls = BTreeMap::new();
    let mut ordered_regions = BTreeMap::new();
    for (kind_index, regional_work) in work_by_kind.iter().enumerate() {
        let region_slices = regional_work
            .iter()
            .map(|(region, work)| (*region, work.as_slice()))
            .collect::<Vec<_>>();
        let pressures = pressure_map(region_slices.iter().copied(), CategoryWeights::default())
            .into_values()
            .collect::<Vec<_>>();
        let kind_capacities = capacities
            .iter()
            .filter_map(|((region, candidate_kind), capacity)| {
                (*candidate_kind == kind_index).then_some((*region, *capacity))
            })
            .collect::<Vec<_>>();
        for (source, pull) in outward_pull_budgets(&kind_capacities, &pressures, u32::MAX) {
            pulls.insert((source, kind_index), pull);
        }
        for source in source_regions.keys().copied() {
            let mut ordered = region_slices.clone();
            ordered.sort_by_key(|(region, _)| {
                (region_distance_key(source, *region), region.y, region.x)
            });
            ordered_regions.insert((source, kind_index), ordered);
        }
    }
    let bounds = CandidateBounds {
        max_regions: RUNTIME_MAX_CANDIDATE_REGIONS,
        max_candidates: RUNTIME_MAX_CANDIDATES,
    };

    for bot in candidates {
        let bot_key = (bot.region, kind_index(bot.kind));
        let Some(pull) = pulls.get(&bot_key).copied() else {
            if bot.resume_pending {
                commands.entity(bot.entity).remove::<RegionalLease>();
            }
            continue;
        };
        let Some(ordered) = ordered_regions.get(&bot_key) else {
            continue;
        };
        let decision = if bot.kind == NanobotType::Hauler {
            choose_terminal_logistics_work(
                bot,
                pull,
                ordered,
                bounds,
                &stockpiles,
                facilities,
                chargers,
                &grid,
                &reserved_source,
                &reserved_destination,
                &charger_demand,
                &terminal.ages,
            )
            .map(|opportunity| super::CandidateDecision {
                opportunity,
                regions_examined: 0,
                candidates_examined: 0,
            })
        } else {
            choose_bounded_candidate_from_ordered_regions_with_claims(
                allocation_candidate(bot),
                pull,
                ordered.iter().copied(),
                bounds,
                |work| {
                    let claims = claim_counts
                        .get(&claim_key(work.target))
                        .copied()
                        .unwrap_or(0);
                    if !target_available(
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
                    Some(claims)
                },
            )
        };
        let Some(work) = decision.map(|decision| decision.opportunity) else {
            if bot.resume_pending {
                commands.entity(bot.entity).remove::<RegionalLease>();
            }
            continue;
        };

        if !adapt_decision(
            &mut commands,
            bot,
            work,
            &grid,
            &deposits,
            &mut planned,
            &structures,
            &stockpiles,
            facilities,
            chargers,
            &mut reserved_source,
            &mut reserved_destination,
            &charger_demand,
        ) {
            continue;
        }
        if let OpportunityTarget::Haul { sink, .. } = work.target {
            if facilities.get(sink).is_ok() || chargers.get(sink).is_ok() {
                terminal.ages.waiting.insert(sink, 0);
            }
        }
        let lease = RegionalLease::new(
            work.region,
            work.category,
            work.target,
            work.owner,
            clock.tick(),
            0,
            30,
        );
        commands.entity(bot.entity).insert(lease);
        *claim_counts.entry(claim_key(work.target)).or_insert(0) += 1;
        if let Some(pull) = pulls.get_mut(&bot_key) {
            let remaining = pull.categories.get(work.category).saturating_sub(1);
            pull.categories.set(work.category, remaining);
        }
    }
}

#[derive(Clone, Copy)]
struct TerminalLogisticsScore {
    urgency: u8,
    age_key: u32,
    deficit_key: u64,
    route_cost: f32,
    terminal: u64,
    source: u64,
}

impl TerminalLogisticsScore {
    fn cmp(self, other: Self) -> Ordering {
        self.urgency
            .cmp(&other.urgency)
            .then_with(|| self.age_key.cmp(&other.age_key))
            .then_with(|| self.deficit_key.cmp(&other.deficit_key))
            .then_with(|| self.route_cost.total_cmp(&other.route_cost))
            .then_with(|| self.terminal.cmp(&other.terminal))
            .then_with(|| self.source.cmp(&other.source))
    }
}

#[allow(clippy::too_many_arguments)]
fn choose_terminal_logistics_work(
    bot: BotSnapshot,
    pull: super::RegionalPullBudget,
    ordered: &[(AllocationRegion, &[ActionableOpportunity])],
    bounds: CandidateBounds,
    stockpiles: &Query<(&Stockpile, &Transform)>,
    facilities: &Query<(&ProductionFacility, &Transform)>,
    chargers: &Query<(&Charger, &Transform)>,
    grid: &IntentGrid,
    reserved_source: &BTreeMap<Entity, u32>,
    reserved_destination: &BTreeMap<Entity, u32>,
    charger_demand: &BTreeMap<Entity, (u8, u32)>,
    ages: &TerminalDemandAges,
) -> Option<ActionableOpportunity> {
    if pull.categories.get(OpportunityCategory::Haul) == 0 {
        return None;
    }
    let mut examined = 0;
    let mut best: Option<(TerminalLogisticsScore, ActionableOpportunity)> = None;
    for (_, opportunities) in ordered.iter().take(bounds.max_regions) {
        for work in *opportunities {
            if examined == bounds.max_candidates {
                break;
            }
            examined += 1;
            let OpportunityTarget::Haul { source, sink, .. } = work.target else {
                continue;
            };
            if work.owner.is_some_and(|owner| owner != bot.swarm) {
                continue;
            }
            let Ok((source_state, source_transform)) = stockpiles.get(source) else {
                continue;
            };
            let source_available = source_state
                .amount
                .saturating_sub(reserved_source.get(&source).copied().unwrap_or_default());
            let incoming = reserved_destination.get(&sink).copied().unwrap_or_default();
            let (base_urgency, destination_available, deficit, capacity, sink_pos) =
                if let Ok((facility, transform)) = facilities.get(sink) {
                    let available = facility.input_free_space().saturating_sub(incoming);
                    let amount = HAULER_CARRY_CAPACITY.min(source_available).min(available);
                    let reaches_cycle = facility
                        .input_amount
                        .saturating_add(incoming)
                        .saturating_add(amount)
                        >= PRODUCTION_COST_PER_BOT;
                    (
                        if reaches_cycle { 3 } else { 4 },
                        available,
                        available,
                        facility.input_capacity,
                        transform.translation.truncate(),
                    )
                } else if let Ok((charger, transform)) = chargers.get(sink) {
                    let available = charger.free_space().saturating_sub(incoming);
                    let (emergency_urgency, total_need) =
                        charger_demand.get(&sink).copied().unwrap_or((4, 0));
                    let emergency_remaining = total_need
                        .saturating_sub(charger.amount)
                        .saturating_sub(incoming);
                    let emergency = emergency_urgency < 4 && emergency_remaining > 0;
                    (
                        if emergency { emergency_urgency } else { 4 },
                        if emergency {
                            available.min(emergency_remaining)
                        } else {
                            available
                        },
                        available,
                        charger.capacity,
                        transform.translation.truncate(),
                    )
                } else if let Ok((stockpile, transform)) = stockpiles.get(sink) {
                    let available = stockpile.free_space().saturating_sub(incoming);
                    (
                        5,
                        available,
                        available,
                        stockpile.capacity,
                        transform.translation.truncate(),
                    )
                } else {
                    continue;
                };
            let amount = HAULER_CARRY_CAPACITY
                .min(source_available)
                .min(destination_available);
            if amount == 0 {
                continue;
            }
            let age = ages.waiting_ticks(sink);
            let urgency =
                base_urgency.saturating_sub((age / TERMINAL_FAIRNESS_PROMOTION_TICKS) as u8);
            let deficit_ratio =
                u64::from(deficit).saturating_mul(1_000_000) / u64::from(capacity.max(1));
            let source_pos = source_transform.translation.truncate();
            let score = TerminalLogisticsScore {
                urgency,
                age_key: u32::MAX - age,
                deficit_key: u64::MAX - deficit_ratio,
                route_cost: hauler_route_cost(bot.position, source_pos, grid, bot.swarm)
                    + hauler_route_cost(source_pos, sink_pos, grid, bot.swarm),
                terminal: sink.to_bits(),
                source: source.to_bits(),
            };
            if best
                .as_ref()
                .is_none_or(|(current, _)| score.cmp(*current).is_lt())
            {
                best = Some((score, *work));
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
            | (NanobotType::Defender, OpportunityCategory::Defend)
            | (NanobotType::Hauler, OpportunityCategory::Haul)
    )
}

fn allocation_candidate(bot: BotSnapshot) -> AllocationCandidate {
    let eligibility = match bot.kind {
        NanobotType::Worker => CategoryEligibility::worker(),
        NanobotType::Defender => CategoryEligibility::only(OpportunityCategory::Defend),
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
    bot: BotSnapshot,
    work: ActionableOpportunity,
    claims: usize,
    planned_workers: &BTreeMap<u64, Option<u64>>,
    deposits: &Query<(&ResourceDeposit, &Transform)>,
    structures: &Query<&Transform>,
    stockpiles: &Query<(&Stockpile, &Transform)>,
) -> bool {
    if work.category != OpportunityCategory::Defend && claims >= opportunity_capacity(work) {
        return false;
    }
    match work.target {
        OpportunityTarget::Gather { deposit, .. } => deposits.get(deposit).is_ok(),
        OpportunityTarget::PlannedBuild { structure, .. } => planned_workers
            .get(&structure.to_bits())
            .is_some_and(|worker| worker.is_none() || *worker == Some(bot.entity.to_bits())),
        OpportunityTarget::Maintenance { structure } => structures.get(structure).is_ok(),
        OpportunityTarget::Defend { .. } => true,
        OpportunityTarget::Haul { source, .. } => stockpiles
            .get(source)
            .is_ok_and(|(stockpile, _)| stockpile.amount > 0 && work.available_work > 0),
    }
}

fn opportunity_capacity(work: ActionableOpportunity) -> usize {
    let units = match work.category {
        OpportunityCategory::Gather => work.available_work.div_ceil(WORKER_CARRY_CAPACITY),
        OpportunityCategory::PlannedBuild | OpportunityCategory::Maintenance => 1,
        OpportunityCategory::Defend => work.available_work,
        OpportunityCategory::Haul => work.available_work.div_ceil(HAULER_CARRY_CAPACITY),
    };
    units.max(1) as usize
}

#[allow(clippy::too_many_arguments)]
fn adapt_decision(
    commands: &mut Commands,
    bot: BotSnapshot,
    work: ActionableOpportunity,
    grid: &IntentGrid,
    deposits: &Query<(&ResourceDeposit, &Transform)>,
    planned: &mut Query<(Entity, &mut PlannedStructure, &Transform)>,
    structures: &Query<&Transform>,
    stockpiles: &Query<(&Stockpile, &Transform)>,
    facilities: &Query<(&ProductionFacility, &Transform)>,
    chargers: &Query<(&Charger, &Transform)>,
    reserved_source: &mut BTreeMap<Entity, u32>,
    reserved_destination: &mut BTreeMap<Entity, u32>,
    charger_demand: &BTreeMap<Entity, (u8, u32)>,
) -> bool {
    match work.target {
        OpportunityTarget::Gather { deposit, cell } => {
            let Ok((deposit_state, transform)) = deposits.get(deposit) else {
                return false;
            };
            commands.entity(bot.entity).insert((
                GatherAssignment::new(cell, deposit),
                DirectMovementComponent {
                    xy: transform.translation.truncate(),
                    stop_radius: deposit_state.radius,
                },
            ));
        }
        OpportunityTarget::PlannedBuild { structure, .. } => {
            let Ok((_, mut planned_state, transform)) = planned.get_mut(structure) else {
                return false;
            };
            if planned_state.active_worker.is_some()
                && planned_state.active_worker != Some(bot.entity)
            {
                return false;
            }
            planned_state.active_worker = Some(bot.entity);
            commands.entity(bot.entity).insert((
                PlannedStructureClaim {
                    cell: work.cell,
                    target: structure,
                },
                DirectMovementComponent {
                    xy: transform.translation.truncate(),
                    stop_radius: BUILDING_FOOTPRINT_RADIUS,
                },
            ));
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
                DirectMovementComponent {
                    xy: transform.translation.truncate(),
                    stop_radius: BUILDING_FOOTPRINT_RADIUS,
                },
            ));
        }
        OpportunityTarget::Defend { cell } => {
            let target = Vec2::new(
                (cell.x as f32 + 0.5) * ZONE_BLOCK_SIZE,
                (cell.y as f32 + 0.5) * ZONE_BLOCK_SIZE,
            );
            commands.entity(bot.entity).insert((
                DefendAssignment { cell },
                DirectMovementComponent {
                    xy: target,
                    stop_radius: DEFEND_IN_CELL_STOP_RADIUS,
                },
            ));
        }
        OpportunityTarget::Haul {
            source,
            sink,
            kind: ResourceKind::Minerals,
        } => {
            let Ok((source_state, transform)) = stockpiles.get(source) else {
                return false;
            };
            let sink_free_space = stockpiles
                .get(sink)
                .map(|(stockpile, _)| stockpile.free_space())
                .or_else(|_| {
                    facilities
                        .get(sink)
                        .map(|(facility, _)| facility.input_free_space())
                })
                .or_else(|_| chargers.get(sink).map(|(charger, _)| charger.free_space()))
                .unwrap_or(0);
            let source_available = source_state
                .amount
                .saturating_sub(reserved_source.get(&source).copied().unwrap_or_default());
            let incoming = reserved_destination.get(&sink).copied().unwrap_or_default();
            let mut destination_available = sink_free_space.saturating_sub(incoming);
            if let Ok((charger, _)) = chargers.get(sink) {
                let (urgency, total_need) = charger_demand.get(&sink).copied().unwrap_or((4, 0));
                let emergency_remaining = total_need
                    .saturating_sub(charger.amount)
                    .saturating_sub(incoming);
                if urgency < 4 && emergency_remaining > 0 {
                    destination_available = destination_available.min(emergency_remaining);
                }
            }
            let amount = HAULER_CARRY_CAPACITY
                .min(source_available)
                .min(destination_available);
            if amount == 0 {
                return false;
            }
            let (route, movement) = planned_route_movement(
                bot.position,
                transform.translation.truncate(),
                grid,
                bot.swarm,
                source_state.radius,
            );
            commands.entity(bot.entity).insert((
                HaulerAssignment { source, sink },
                LogisticsReservation::new(source, sink, ResourceKind::Minerals, amount),
                route,
                movement,
            ));
            *reserved_source.entry(source).or_default() += amount;
            *reserved_destination.entry(sink).or_default() += amount;
        }
    }
    true
}

fn region_distance_key(left: AllocationRegion, right: AllocationRegion) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y)
}

fn claim_key(target: OpportunityTarget) -> (u8, u64, u64, u64) {
    match target {
        OpportunityTarget::Gather { deposit, .. } => (0, deposit.to_bits(), 0, 0),
        OpportunityTarget::PlannedBuild { structure, .. } => (1, structure.to_bits(), 0, 0),
        OpportunityTarget::Maintenance { structure } => (2, structure.to_bits(), 0, 0),
        OpportunityTarget::Defend { cell } => {
            (3, i64::from(cell.x) as u64, i64::from(cell.y) as u64, 0)
        }
        OpportunityTarget::Haul { source, sink, .. } => (4, source.to_bits(), sink.to_bits(), 0),
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

        assert_eq!(claim_key(first), claim_key(second));
    }
}
