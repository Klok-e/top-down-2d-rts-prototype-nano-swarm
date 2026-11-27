//! Read-only world observation and owner-scoped application of strategic intent.

mod mine_pressure;
mod mines;

use bevy::prelude::*;
use mine_pressure::MinePressureHistory;
use mines::MineHistory;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
};

use crate::{
    intent::IntentGrid,
    nanobot::{
        Cargo, Charge, Charger, ChargerAssignment, ChargerProgress, ClearingEvacuation, Commitment,
        DefenderDisengagement, DefenderResponse, DefenderRout, DefenderStagingAssignments,
        DirectMovementComponent, DropOffArea, ExtractProgress, GatherAssignment, Health,
        InteractionRegion, LogisticsReservation, MatchOutcome, Nanobot, NanobotType, OwnerSwarm,
        PlannedStructure, ProductionFacility, RegionalAllocationSet, ResolvedCombatFact,
        ReturningToStockpile, Structure, Swarm, SwarmId, SwarmMember, WorkBlocked,
        effective_attack, gather_worker_slots,
    },
    navigation::{Navigation, Obstacle, RouteGoal, RouteOutcome},
    reachability::{IntentWriteAction, SwarmReachabilityMap, apply_reachable_intent},
    resources::{
        ResourceDeposit, ResourceKind, ResourceLedger, SinkUpkeep, Stockpile, StockpileRole,
        buffered_minerals,
    },
    strategic_controller::{
        BotState, BuildFootprint, Controller, DefenderDuty, DepositState, GameState,
        IntentEditAction, MineObservationAt, StructureKind, StructureState, SwarmState,
    },
    terrain::RockFormation,
};

#[derive(Component, Debug)]
pub struct StrategicController {
    planner: Controller,
}

impl StrategicController {
    pub fn new(owner: SwarmId) -> Self {
        Self {
            planner: Controller::new(owner),
        }
    }

    pub fn owner(&self) -> SwarmId {
        self.planner.owner()
    }
}

pub struct StrategicControllerPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct StrategicControllerSet;

impl Plugin for StrategicControllerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::tuning::Tuning>()
            .init_resource::<ControllerTelemetry>()
            .add_message::<ResolvedCombatFact>()
            .add_systems(
                FixedUpdate,
                strategic_intent_system
                    .in_set(StrategicControllerSet)
                    .after(crate::nanobot::NanobotSimulationSet::Movement)
                    .before(RegionalAllocationSet::Project)
                    .run_if(resource_exists::<SwarmReachabilityMap>),
            );
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ControllerProfile {
    pub reviews: u64,
    pub intent_edits: u64,
    pub work_units: u64,
    pub last_explanation: String,
}

#[derive(Resource, Default)]
pub struct ControllerTelemetry {
    profiles: BTreeMap<u32, ControllerProfile>,
}

impl ControllerTelemetry {
    pub fn profiles(&self) -> BTreeMap<u32, ControllerProfile> {
        self.profiles.clone()
    }

    fn record_review(
        &mut self,
        owner: u32,
        intent_edits: usize,
        work_units: usize,
        explanation: String,
    ) {
        let profile = self.profiles.entry(owner).or_default();
        profile.reviews += 1;
        profile.intent_edits += intent_edits as u64;
        profile.work_units += work_units as u64;
        profile.last_explanation = explanation;
    }
}

#[derive(Default)]
struct StrategicRouteCache {
    revision: u64,
    results: VecDeque<(Vec2, RouteGoal, RouteOutcome)>,
}

impl StrategicRouteCache {
    /// Adopt a new navigation revision, keeping only found routes whose whole path is still
    /// movement-clear. A kept route may be longer than a fresh search would find; unreachable
    /// outcomes could have been invalidated by removed obstacles, so they are dropped.
    fn revalidate(&mut self, navigation: &Navigation) {
        self.revision = navigation.revision();
        self.results.retain(|(start, _, outcome)| match outcome {
            RouteOutcome::Found(route) => route.still_clear(navigation, *start),
            RouteOutcome::Unreachable => false,
        });
    }

    fn query(
        &mut self,
        navigation: &Navigation,
        start: Vec2,
        goal: RouteGoal,
        allow_work: &mut dyn FnMut() -> bool,
    ) -> Option<RouteOutcome> {
        const CACHE_LIMIT: usize = 64;
        for (cached_start, cached_goal, outcome) in &self.results {
            if !allow_work() {
                return None;
            }
            if *cached_start == start && *cached_goal == goal {
                return Some(outcome.clone());
            }
        }
        let outcome = navigation.route_without_paint(start, goal, allow_work)?;
        if self.results.len() == CACHE_LIMIT {
            self.results.pop_front();
        }
        self.results.push_back((start, goal, outcome.clone()));
        Some(outcome)
    }
}

#[derive(Default)]
struct MineralIncomeHistory {
    samples: VecDeque<(u64, u64)>,
}

impl MineralIncomeHistory {
    fn per_second(&mut self, tick: u64, extracted: u64) -> f32 {
        const TICKS_PER_SECOND: u64 = crate::SIMULATION_HZ as u64;
        const INCOME_WINDOW_SECONDS: u64 = 8;

        if self
            .samples
            .back()
            .is_some_and(|&(previous_tick, previous_total)| {
                tick < previous_tick || extracted < previous_total
            })
        {
            self.samples.clear();
        }
        if self.samples.back().is_none_or(|&(previous_tick, _)| {
            tick.saturating_sub(previous_tick) >= TICKS_PER_SECOND
        }) {
            self.samples.push_back((tick, extracted));
        }
        while self.samples.len() > 1
            && tick.saturating_sub(self.samples[1].0) >= INCOME_WINDOW_SECONDS * TICKS_PER_SECOND
        {
            self.samples.pop_front();
        }

        let (start, total) = self.samples[0];
        let elapsed_seconds = (tick - start) as f32 / TICKS_PER_SECOND as f32;
        (extracted - total) as f32 / elapsed_seconds.max(1.0)
    }
}

#[derive(Default)]
struct StrategicObservations {
    routes: StrategicRouteCache,
    previous_positions: HashMap<Entity, Vec2>,
    mines: HashMap<(SwarmId, Entity), MineHistory>,
    active_workers: HashMap<(SwarmId, Entity), u32>,
    mine_pressure: MinePressureHistory,
}

#[derive(Clone, Copy)]
struct SourceObservation {
    id: Entity,
    owner: Option<SwarmId>,
    position: Vec2,
    interaction: InteractionRegion,
    room: u32,
    capacity: u32,
}

#[derive(Default, Clone, Copy)]
struct EconomyObservation {
    banked_minerals: u32,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn strategic_intent_system(
    tuning: Res<crate::tuning::Tuning>,
    mut controllers: Query<(Entity, &SwarmId, &mut StrategicController)>,
    swarms: Query<(Entity, &SwarmId, Option<&Transform>), With<Swarm>>,
    bots: Query<
        (
            Entity,
            &SwarmMember,
            &NanobotType,
            &Transform,
            &Health,
            Option<&Charge>,
            Option<&Cargo>,
            (
                Option<&GatherAssignment>,
                Option<&LogisticsReservation>,
                Has<ExtractProgress>,
                Has<ReturningToStockpile>,
                Has<DirectMovementComponent>,
                Has<WorkBlocked>,
            ),
            (
                Option<&Commitment>,
                Has<ChargerAssignment>,
                Has<ChargerProgress>,
                Has<DefenderDisengagement>,
                Has<DefenderRout>,
                Has<ClearingEvacuation>,
                Option<&DefenderResponse>,
            ),
        ),
        With<Nanobot>,
    >,
    structures: Query<
        (
            Entity,
            &Transform,
            Option<&OwnerSwarm>,
            Option<&Structure>,
            Option<&ProductionFacility>,
            Option<&Stockpile>,
            Option<&StockpileRole>,
            Option<&Charger>,
            Option<&PlannedStructure>,
            Option<&SinkUpkeep>,
        ),
        Or<(
            With<Structure>,
            With<ProductionFacility>,
            With<Stockpile>,
            With<Charger>,
            With<PlannedStructure>,
        )>,
    >,
    deposits: Query<(Entity, &Transform, &ResourceDeposit, Option<&OwnerSwarm>)>,
    rocks: Query<(&RockFormation, &Transform)>,
    ledger: Res<ResourceLedger>,
    mut grid: ResMut<IntentGrid>,
    reachability: Res<SwarmReachabilityMap>,
    (navigation, staging_assignments): (Res<Navigation>, Option<Res<DefenderStagingAssignments>>),
    (time, outcome): (Res<Time<Fixed>>, Option<Res<MatchOutcome>>),
    mut combat: MessageReader<ResolvedCombatFact>,
    mut telemetry: ResMut<ControllerTelemetry>,
    mut observations: Local<StrategicObservations>,
    mut income_history: Local<BTreeMap<Entity, MineralIncomeHistory>>,
) {
    if controllers.is_empty()
        || outcome
            .as_deref()
            .is_some_and(|outcome| *outcome != MatchOutcome::InProgress)
    {
        combat.clear();
        observations.mine_pressure = MinePressureHistory::default();
        return;
    }
    let StrategicObservations {
        routes,
        previous_positions,
        mines,
        active_workers,
        mine_pressure,
    } = &mut *observations;
    let owners: BTreeMap<_, _> = swarms.iter().map(|(entity, id, _)| (entity, *id)).collect();
    let tick = (time.elapsed_secs_f64() * crate::SIMULATION_HZ).round() as u64;
    if mine_pressure.begin_tick(&owners, tick) {
        combat.clear();
    }
    for fact in combat.read() {
        if let ResolvedCombatFact::Hit(hit) = fact {
            mine_pressure.record_hit(tick, *hit);
        }
    }
    mine_pressure.clear_links();
    income_history.retain(|entity, _| owners.contains_key(entity));
    previous_positions.retain(|entity, _| bots.get(*entity).is_ok());
    active_workers.clear();
    let mut bot_states: Vec<_> = bots
        .iter()
        .map(
            |(
                entity,
                owner,
                kind,
                transform,
                health,
                charge,
                cargo,
                gathering,
                (
                    commitment,
                    assigned_charger,
                    charging,
                    disengaging,
                    routing,
                    evacuating,
                    response,
                ),
            )| {
                let (assignment, reservation, extracting, returning, moving, blocked) = gathering;
                if *kind == NanobotType::Worker && health.current > 0 && !blocked {
                    let deposit = if returning && cargo.is_some_and(|cargo| cargo.amount > 0) {
                        reservation.map(|reservation| reservation.source)
                    } else if extracting || moving {
                        assignment.map(|assignment| assignment.deposit)
                    } else {
                        None
                    };
                    if let Some(deposit) = deposit.filter(|&deposit| deposits.contains(deposit)) {
                        *active_workers.entry((owner.0, deposit)).or_default() += 1;
                        mine_pressure.link_mine_work(entity, deposit);
                        if let Some(reservation) = reservation.filter(|r| r.source == deposit) {
                            mine_pressure.link_mine_work(reservation.destination, deposit);
                        }
                    }
                }
                let position = transform.translation.truncate();
                if *kind == NanobotType::Defender
                    && health.current > 0
                    && effective_attack(charge.map_or(0.0, |charge| charge.current), &tuning) > 0.0
                {
                    mine_pressure.record_defender(entity, owner.0, position);
                }
                let previous = previous_positions
                    .insert(entity, position)
                    .unwrap_or(position);
                let defender_duty = if *kind != NanobotType::Defender {
                    DefenderDuty::Available
                } else if commitment.is_some_and(|task| *task != Commitment::Idle)
                    || assigned_charger
                    || charging
                    || disengaging
                    || routing
                    || evacuating
                {
                    DefenderDuty::Unavailable
                } else if let Some(response) = response {
                    let response_position = bots
                        .get(response.target)
                        .map(|(_, _, _, transform, ..)| transform.translation.truncate())
                        .or_else(|_| {
                            structures
                                .get(response.target)
                                .map(|(_, transform, ..)| transform.translation.truncate())
                        })
                        .ok();
                    response_position.map_or(DefenderDuty::Available, DefenderDuty::RespondingTo)
                } else {
                    staging_assignments
                        .as_ref()
                        .and_then(|assignments| assignments.destination(entity))
                        .map_or(DefenderDuty::Available, DefenderDuty::StagingAt)
                };
                BotState {
                    id: entity.to_bits(),
                    owner: owner.0,
                    kind: *kind,
                    position,
                    health: health.current,
                    max_health: health.max,
                    charge: charge.map_or(0.0, |c| c.current),
                    max_charge: charge.map_or(0.0, |c| c.max),
                    cargo: cargo.map_or(0, |c| c.amount),
                    displacement: position - previous,
                    defender_duty,
                }
            },
        )
        .collect();
    for &key in active_workers.keys() {
        mines.entry(key).or_default();
    }
    mines.retain(|&(owner, deposit), history| {
        let Ok((_, _, _, exclusive)) = deposits.get(deposit) else {
            return false;
        };
        if !swarm_exists(&owners, owner, exclusive) {
            return false;
        }
        let workers = active_workers.get(&(owner, deposit)).copied().unwrap_or(0);
        history.observe(tick, workers, ledger.worker_deliveries_for(owner, deposit));
        true
    });
    bot_states.sort_by_key(|bot| bot.id);
    let mut economies = BTreeMap::<SwarmId, EconomyObservation>::new();
    let mut operational_sources = Vec::new();
    let mut build_footprints = Vec::new();
    let mut structure_states: Vec<_> = structures
        .iter()
        .filter_map(
            |(
                entity,
                transform,
                owner,
                condition,
                facility,
                stockpile,
                role,
                charger,
                plan,
                upkeep,
            )| {
                let owner = match owner {
                    Some(owner) => Some(*owners.get(&owner.0)?),
                    None => None,
                };
                let kind = if plan.is_some() {
                    StructureKind::Planned
                } else if facility.is_some() {
                    StructureKind::Facility
                } else if charger.is_some() {
                    StructureKind::Charger
                } else if stockpile.is_some() {
                    if role == Some(&StockpileRole::Sink) {
                        StructureKind::Sink
                    } else {
                        StructureKind::Source
                    }
                } else {
                    StructureKind::Other
                };
                let minerals = buffered_minerals(facility, stockpile, charger);
                let operational = plan.is_none() && condition.is_none_or(Structure::is_operational);
                if operational
                    && kind == StructureKind::Source
                    && let Some(stockpile) = stockpile.filter(|stockpile| {
                        stockpile.kind == ResourceKind::Minerals && stockpile.capacity > 0
                    })
                {
                    operational_sources.push(SourceObservation {
                        id: entity,
                        owner,
                        position: transform.translation.truncate(),
                        interaction: InteractionRegion::structure(transform),
                        room: stockpile.free_space(),
                        capacity: stockpile.capacity,
                    });
                }
                let owner = owner?;
                let health = condition.map_or(tuning.construction.max_health, |c| c.health);
                let needs_build = match kind {
                    StructureKind::Planned => plan.is_some_and(|plan| {
                        plan.kind.placement_intent() == crate::intent::IntentKind::Build
                    }),
                    StructureKind::Facility => true,
                    StructureKind::Sink => {
                        SinkUpkeep::paid(upkeep)
                            || stockpile.is_some_and(|stockpile| stockpile.amount > 0)
                    }
                    _ => false,
                };
                if health > 0 && needs_build {
                    let (center, half) = Obstacle::structure(transform).bounds();
                    build_footprints.push(BuildFootprint {
                        owner,
                        center,
                        half,
                    });
                }
                if operational {
                    let economy = economies.entry(owner).or_default();
                    economy.banked_minerals = economy.banked_minerals.saturating_add(minerals);
                }
                Some(StructureState {
                    id: entity.to_bits(),
                    owner,
                    position: transform.translation.truncate(),
                    kind,
                    operational,
                    health,
                    max_health: tuning.construction.max_health,
                    minerals,
                })
            },
        )
        .collect();
    structure_states.sort_by_key(|structure| structure.id);
    operational_sources.sort_by_key(|source| source.id.to_bits());
    let mut swarm_states: Vec<_> = swarms
        .iter()
        .map(|(entity, id, position)| {
            let economy = economies.get(id).copied().unwrap_or_default();
            SwarmState {
                id: *id,
                home: position.map_or(Vec2::ZERO, |t| t.translation.truncate()),
                minerals: economy.banked_minerals,
                mineral_income_per_second: income_history
                    .entry(entity)
                    .or_default()
                    .per_second(tick, ledger.extracted_minerals_for(*id)),
            }
        })
        .collect();
    swarm_states.sort_by_key(|swarm| swarm.id);
    let mut deposit_states: Vec<_> = deposits
        .iter()
        .filter_map(|(entity, transform, deposit, owner)| {
            let owner = match owner {
                Some(owner) => Some(*owners.get(&owner.0)?),
                None => None,
            };
            Some(DepositState {
                id: entity.to_bits(),
                owner,
                position: transform.translation.truncate(),
                amount: deposit.amount,
                radius: deposit.radius,
            })
        })
        .collect();
    deposit_states.sort_by_key(|deposit| deposit.id);
    mine_pressure.update_presence(tick, &deposit_states, tuning.combat.attack_range);
    let terrain: Vec<_> = rocks
        .iter()
        .map(|(rock, transform)| rock.obstacle(transform))
        .collect();
    if reachability.is_changed() {
        routes.revision = navigation.revision();
        routes.results.clear();
    } else if routes.revision != navigation.revision() {
        routes.revalidate(&navigation);
    }
    let routes = RefCell::new(routes);
    let route_query = |start, goal, allow_work: &mut dyn FnMut() -> bool| {
        routes
            .borrow_mut()
            .query(&navigation, start, goal, allow_work)
    };
    let mine_query = |owner: SwarmId,
                      deposit: &DepositState,
                      location: MineObservationAt,
                      allow_work: &mut dyn FnMut() -> bool| {
        if !allow_work() {
            return None;
        }
        let (mut observation, delivery_room) = mines
            .get(&(owner, Entity::from_bits(deposit.id)))
            .map_or_else(Default::default, |history| history.recent(tick));
        let gather_cell = match location {
            MineObservationAt::Service(gather_cell) => gather_cell,
            MineObservationAt::Defense(cells) => {
                observation.pressure =
                    mine_pressure.observe(tick, owner, deposit, cells, &tuning, allow_work)?;
                return Some(observation);
            }
        };
        let drop_off_area = DropOffArea::for_deposit(deposit.position, deposit.radius, &tuning);
        let deposit_region = InteractionRegion::deposit(
            &Transform::from_translation(deposit.position.extend(0.0)),
            deposit.radius,
        )
        .within_cell(gather_cell);
        let mut gather_approach = None;
        let mut free_room = 0u32;
        let mut total_capacity = 0u32;
        let ring_limit = gather_worker_slots(deposit.radius);
        let carry_capacity = tuning.logistics.worker_carry_capacity.max(1);
        for source in &operational_sources {
            if !allow_work() {
                return None;
            }
            if source
                .owner
                .is_some_and(|source_owner| source_owner != owner)
                || !drop_off_area.contains(source.position)
            {
                continue;
            }
            let start = if let Some(start) = gather_approach {
                start
            } else {
                let swarm = swarm_states
                    .binary_search_by_key(&owner, |swarm| swarm.id)
                    .ok()?;
                let home = swarm_states[swarm].home;
                let RouteOutcome::Found(route) =
                    route_query(home, RouteGoal::Interaction(deposit_region), allow_work)?
                else {
                    break;
                };
                let start = route.waypoints.last().copied().unwrap_or(home);
                gather_approach = Some(start);
                start
            };
            if navigation
                .connected_interaction_with_budget(start, source.interaction, allow_work)
                .ok()?
                .is_none()
            {
                continue;
            }
            free_room = free_room.saturating_add(source.room);
            total_capacity = total_capacity.saturating_add(source.capacity);
            if free_room.div_ceil(carry_capacity) >= ring_limit {
                break;
            }
        }
        observation.has_source = total_capacity > 0;
        observation.has_space = free_room > 0;
        // Reservations use physical room. Recent deliveries also show how much room
        // hauling can turn over in one Worker cycle even when the buffer is now full.
        let usable_room = free_room.max(delivery_room).min(total_capacity);
        observation.worker_capacity = ring_limit.min(usable_room.div_ceil(carry_capacity));
        Some(observation)
    };
    let state = GameState {
        grid: &grid,
        swarms: &swarm_states,
        bots: &bot_states,
        structures: &structure_states,
        build_footprints: &build_footprints,
        deposits: &deposit_states,
        terrain: &terrain,
        charge: &tuning.charge,
        reachability: Some(&reachability),
        route_query: Some(&route_query),
        mine_query: Some(&mine_query),
        tick,
        finished: false,
    };
    let mut ordered: Vec<_> = controllers
        .iter()
        .map(|(entity, owner, _)| (*owner, entity))
        .collect();
    ordered.sort_by_key(|(owner, entity)| (*owner, entity.to_bits()));
    let mut decisions = Vec::with_capacity(ordered.len());
    for (owner, entity) in ordered {
        let Ok((_, _, mut controller)) = controllers.get_mut(entity) else {
            continue;
        };
        if controller.owner() != owner {
            continue;
        }
        telemetry.profiles.entry(owner.0).or_default();
        let decision = controller.planner.decide(&tuning.autonomy, &state);
        if decision.reviewed {
            telemetry.record_review(
                owner.0,
                decision.edits.len(),
                decision.work_units,
                decision.explanation.clone(),
            );
        } else {
            telemetry.profiles.entry(owner.0).or_default().work_units += decision.work_units as u64;
        }
        decisions.push((owner, decision));
    }
    for (owner, decision) in decisions {
        for edit in decision.edits {
            let action = match edit.action {
                IntentEditAction::Paint => IntentWriteAction::Paint,
                IntentEditAction::Erase => IntentWriteAction::Erase,
            };
            if let Err(error) = apply_reachable_intent(
                &mut grid,
                &reachability,
                owner,
                edit.cell,
                edit.kind,
                action,
            ) {
                log::error!("Strategic Controller emitted rejected intent edit: {error}");
            }
        }
    }
}

fn swarm_exists(
    owners: &BTreeMap<Entity, SwarmId>,
    owner: SwarmId,
    exclusive: Option<&OwnerSwarm>,
) -> bool {
    match exclusive {
        Some(exclusive) => owners.get(&exclusive.0) == Some(&owner),
        None => owners.values().any(|&current| current == owner),
    }
}

/// Ask existing controllers to review changed scores under their retained allowance.
pub fn reconcile_controller_tuning(
    world: &mut World,
    old: &crate::tuning::Tuning,
    new: &crate::tuning::Tuning,
) {
    if old.autonomy == new.autonomy
        && old.combat == new.combat
        && old.charge == new.charge
        && old.construction == new.construction
    {
        return;
    }
    for mut controller in world.query::<&mut StrategicController>().iter_mut(world) {
        controller.planner.request_tuning_review();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::Obstacle;

    #[test]
    fn revalidation_drops_unreachable_outcomes_and_keeps_clear_found_routes() {
        let grid = crate::intent::IntentGrid::new(8, 8);
        let wall = Obstacle::Rectangle {
            center: Vec2::new(1000.0, 0.0),
            half: Vec2::splat(200.0),
        };
        let mut navigation = Navigation::new(&grid, vec![wall]);
        let mut cache = StrategicRouteCache::default();
        let start = Vec2::new(-1000.0, 0.0);
        let free = RouteGoal::Point(Vec2::new(-500.0, 300.0));
        let walled = RouteGoal::Point(Vec2::new(1000.0, 0.0));
        let mut allow = || true;
        assert!(matches!(
            cache.query(&navigation, start, free, &mut allow),
            Some(RouteOutcome::Found(_))
        ));
        assert!(matches!(
            cache.query(&navigation, start, walled, &mut allow),
            Some(RouteOutcome::Unreachable)
        ));
        assert_eq!(cache.results.len(), 2);

        navigation.refresh(&grid, Vec::new());
        cache.revalidate(&navigation);
        assert_eq!(cache.revision, navigation.revision());
        assert_eq!(cache.results.len(), 1);
        assert_eq!(cache.results[0].1, free);
    }
}
