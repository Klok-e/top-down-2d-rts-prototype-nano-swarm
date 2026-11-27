//! Demand-driven production facilities.
//!
//! Production facilities consume delivered resources and fill typed workload
//! shortages. Additional facilities emerge from demand pressure when existing
//! capacity is too busy.
//!
//! ## State machine
//!
//! Each [`ProductionFacility`] cycles through:
//!
//! ```text
//!   Idle (no current_target)
//!      -> pick the greatest relative typed shortage
//!      -> try consume material from the facility hopper
//!      -> on success: Working (current_target set, progress=0)
//!      -> on failure: remain Idle
//!   Working
//!      -> advance progress each tick
//!      -> on reaching the configured cycle budget:
//!         spawn a new nanobot of current_target
//!         reset to Idle
//! ```
//!
//! Material flow: the facility pulls the full the configured cost from
//! its input hopper at the start of a production cycle. Haulers physically fill
//! that hopper through the final logistics leg, and production consumes the
//! committed material up front.
//!
//! Shared cost/time: all three early types (Worker, Hauler,
//! Defender) cost the same number of minerals and take the same
//! number of ticks to produce. Differentiated costs are a
//! follow-up issue per the PRD.

use crate::navigation::Obstacle;
use crate::tuning::Tuning;
use std::collections::HashMap;

use bevy::prelude::*;

use crate::ai::AiStateComponent;
use crate::battle_statistics::{BattleCounters, BattleEvent};
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{Commitment, NanobotType};
use crate::nanobot::components::{Health, Nanobot, Swarm, SwarmId, SwarmMember, VelocityComponent};
use crate::nanobot::maintenance::SupportCondition;
use crate::nanobot::planned::{PlannedKind, planned_visual_components};
use crate::nanobot::{CombatIdentity, NanobotBundle, PlannedStructure, combat_key};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};
use crate::structure_sprites::StructureSprites;

/// Owner-scoped Production Pressure accumulated by each swarm.
#[derive(Debug, Default, Resource)]
pub struct ProductionPressure {
    ticks_by_swarm: HashMap<SwarmId, u32>,
}

impl ProductionPressure {
    pub fn ticks_for(&self, swarm: SwarmId) -> u32 {
        self.ticks_by_swarm.get(&swarm).copied().unwrap_or(0)
    }

    fn set_ticks(&mut self, swarm: SwarmId, ticks: u32) {
        if ticks == 0 {
            self.ticks_by_swarm.remove(&swarm);
        } else {
            self.ticks_by_swarm.insert(swarm, ticks);
        }
    }
}

fn next_production_pressure_ticks(current: u32, pressure_continues: bool, threshold: u32) -> u32 {
    if pressure_continues {
        current.saturating_add(1).min(threshold)
    } else {
        0
    }
}

/// Marker for a [`crate::nanobot::Swarm`] that is driven by
/// prepainted intent. Opponent nanobots still run
/// through the same scoring, logistics, and production systems
/// as the player swarm; the marker only lets callers query
/// opponents separately.
#[derive(Debug, Component, Default)]
pub struct OpponentSwarm {}

/// Ties a production facility to the swarm that owns it. Used
/// to resolve typed demand and to decide which swarm a completed cycle spawns
/// its new nanobot under. Every production facility must carry this marker.
#[derive(Debug, Component, Clone, Copy)]
pub struct OwnerSwarm(pub Entity);

/// An automatic production facility. Spawned by
/// `production_facility_auto_creation_system` near a swarm that
/// has unmet production demand. The facility's own state is
/// carried on this component so multiple facilities can run
/// independently.
#[derive(Debug, Component, Clone)]
pub struct ProductionFacility {
    /// Tick counter within the current production cycle. Reset
    /// to 0 when a new cycle starts; reaches
    /// the configured cycle budget to finish the cycle.
    pub progress: u32,
    /// Simulation tick when output finished; retained until a free exterior exit exists.
    pub finished_at: Option<u64>,
    /// Type currently being produced, or `None` if the facility
    /// is idle and waiting to pick its next target.
    pub current_target: Option<NanobotType>,
    /// Resource kind the input hopper accepts. Always
    /// [`ResourceKind::Minerals`] in the first implementation;
    /// kept as a field so the hauler sink matcher can pair it
    /// against the hauler's carried kind without a hardcoded
    /// assumption.
    pub input_kind: ResourceKind,
    /// Material currently sitting in the facility's input hopper.
    /// Haulers (logistics leg 3) deliver into this buffer;
    /// production pulls the configured cost from it at
    /// the start of each cycle. This is the ONLY buffer
    /// production consumes from -- a sink stockpile no longer
    /// feeds production directly, so the three-leg chain is
    /// real and the hauler cannot be bypassed.
    pub input_amount: u32,
    /// Maximum material the input hopper can hold. Haulers may still claim
    /// queued space beyond it (see [`Self::claimable_capacity`]), but unload
    /// only into its real free space.
    pub input_capacity: u32,
    /// The owner swarm had unmet Population Demand when targets were last
    /// picked, so the next funded cycle starts as soon as the hopper holds its
    /// cost. Lets haulers claim queued space even while the facility idles.
    pub demand_waiting: bool,
    /// Space beyond the hopper Haulers may claim while the facility is busy:
    /// what it consumes during one delivery's lead time, in whole Hauler
    /// loads. Kept by [`facility_delivery_window_system`]; 0 until first
    /// sized. [`Self::claimable_capacity`] never offers less than one cycle.
    pub queued_space: u32,
    /// Fixed ticks from a Hauler claiming a delivery here to arriving with it,
    /// averaged over recent deliveries; `None` until one arrives. Kept by
    /// [`facility_lead_time_system`].
    pub measured_lead_ticks: Option<f32>,
}

impl ProductionFacility {
    /// New idle facility with an empty input hopper, used by the
    /// auto-creation promotion path and by tests. A facility
    /// starts idle and stays idle until a hauler delivers enough
    /// material for at least one production cycle.
    pub fn new() -> Self {
        Self::with_tuning(&Tuning::default())
    }

    pub fn with_tuning(tuning: &Tuning) -> Self {
        Self {
            progress: 0,
            finished_at: None,
            current_target: None,
            input_kind: ResourceKind::Minerals,
            input_amount: 0,
            input_capacity: tuning.production.input_capacity,
            demand_waiting: false,
            queued_space: 0,
            measured_lead_ticks: None,
        }
    }

    /// True when the facility is currently producing a nanobot.
    /// Used by the auto-creation system to detect "all existing
    /// facilities are too busy".
    pub fn is_busy(&self, ticks_per_bot: u32) -> bool {
        self.current_target.is_some() && self.progress < ticks_per_bot
    }

    /// Free capacity in the input hopper for hauler delivery.
    /// Mirrors [`crate::resources::Stockpile::free_space`] and
    /// [`crate::nanobot::Charger::free_space`] so the hauler
    /// sink selection treats all three terminal/buffer kinds
    /// through the same shape.
    pub fn input_free_space(&self) -> u32 {
        self.input_capacity.saturating_sub(self.input_amount)
    }

    /// Hopper capacity haulers may claim. While the facility is producing or
    /// its swarm still wants nanobots, its delivery window ([`Self::queued_space`],
    /// at least one production cycle) is added: Haulers holding that claim
    /// travel or wait while cycles drain the hopper, so the next load is
    /// already on its way when the hopper empties.
    pub fn claimable_capacity(&self, tuning: &Tuning) -> u32 {
        let queued = if self.current_target.is_some() || self.demand_waiting {
            self.queued_space.max(tuning.production.cost_per_bot)
        } else {
            0
        };
        self.input_capacity.saturating_add(queued)
    }

    /// Unfilled part of [`Self::claimable_capacity`]. Unloading still moves
    /// minerals only into [`Self::input_free_space`].
    pub fn claimable_space(&self, tuning: &Tuning) -> u32 {
        self.claimable_capacity(tuning)
            .saturating_sub(self.input_amount)
    }
}

impl Default for ProductionFacility {
    fn default() -> Self {
        Self::new()
    }
}

/// Cycle progress for a [`ProductionFacility`] as an
/// integer percent in `[0, 100]`. An idle facility
/// (`current_target = None`) reports 0% so the label
/// formatter does not have to special-case it. A working
/// facility's percent is elapsed ticks / cycle ticks
/// floored to an integer; the label uses this directly.
///
/// The function is pure and lives next to the
/// production data so unit tests can pin the contract
/// without a Bevy `App`. The structure-overlay module
/// uses it through the `crate::nanobot` re-export.
pub fn production_progress_percent(facility: &ProductionFacility, ticks_per_bot: u32) -> u32 {
    if facility.current_target.is_none() {
        return 0;
    }
    if ticks_per_bot == 0 {
        return 100;
    }
    let pct = (facility.progress as u64 * 100 / ticks_per_bot as u64) as u32;
    pct.min(100)
}

/// Count nanobots that belong to `swarm_id`'s swarm, keyed by
/// type. Used by the per-swarm production systems to measure
/// only the population that the swarm owns, so an opponent
/// swarm's shortage count is not muddied by the player swarm's
/// nanobots.
///
/// Issue #38 / ADR-0004: nanobots are top-level entities,
/// not children of the swarm. The function looks up every
/// `Nanobot` whose `SwarmMember` matches the supplied
/// `SwarmId`. The previous `Entity` + `Children` based
/// signature is replaced with a `SwarmId` based signature
/// so the function does not need to re-query the swarm's
/// own components on every call. Callers already have the
/// `SwarmId` from the swarm-iteration query.
pub fn count_swarm_nanobots_by_type(
    swarm_id: SwarmId,
    nanobots: &Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
) -> HashMap<NanobotType, u32> {
    let mut counts = HashMap::new();
    for (ty, member) in nanobots.iter() {
        if member.0 == swarm_id {
            *counts.entry(*ty).or_insert(0) += 1;
        }
    }
    counts
}

/// Plan a new production facility from typed workload demand pressure. For
/// each swarm in the world, a [`PlannedStructure`] of
/// [`PlannedKind::ProductionFacility`] emerges when:
///
/// 1. the swarm has a typed population shortage, AND
/// 2. every operational facility that belongs to this swarm stays busy for
///    the configured pressure window consecutive ticks, AND
/// 3. the swarm owns at least one `Build`-painted cell that
///    does not already host a planned or completed
///    structure. The Build Zone is the placement constraint
///    (issue #27 acceptance: "Planned Production Facility
///    placement is constrained to an owned Build Zone").
///
/// The plan itself does NOT consume any material: build work is
/// worker-time-only in v1, so a Worker can build the plan
/// even when the swarm is short on minerals. The completed
/// `ProductionFacility` chooses from current demand after physical funding.
///
/// Acceptance: "No new Production Facility is planned when
/// no suitable Build Zone exists." A swarm without any
/// owned Build cells cannot plan a Production Facility, so
/// the auto-creation is a no-op for that swarm. This is the
/// "Build Zone constrains placement" half of the contract.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn production_facility_auto_creation_system(
    mut commands: Commands,
    access: super::construction_access::ConstructionAccess,
    grid: Res<IntentGrid>,
    structure_sprites: Res<StructureSprites>,
    tuning: Res<Tuning>,
    population_demand: Res<crate::nanobot::PopulationDemand>,
    mut pressure: ResMut<ProductionPressure>,
    nanobots: Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
    facilities: Query<(
        Entity,
        &ProductionFacility,
        &OwnerSwarm,
        Option<&SupportCondition>,
    )>,
    existing_targets: Query<
        &Transform,
        Or<(
            With<PlannedStructure>,
            With<ProductionFacility>,
            With<Stockpile>,
            With<crate::nanobot::Charger>,
        )>,
    >,
    planned_facilities: Query<(&PlannedStructure, &OwnerSwarm)>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    let mut access_layout = access.snapshot();
    let mut obstacles: Vec<Obstacle> = deposits
        .iter()
        .map(|(deposit, transform)| {
            Obstacle::deposit(transform.translation.truncate(), deposit.radius)
        })
        .collect();
    for transform in &existing_targets {
        obstacles.push(Obstacle::structure(transform));
    }
    // Physical footprints constrain placement within every swarm-owned Build cell.
    let mut build_cells_by_swarm: HashMap<SwarmId, Vec<IVec2>> = HashMap::new();
    for (cell, intent_cell) in grid.iter_active_cells() {
        if !intent_cell.has(IntentKind::Build) {
            continue;
        }
        for owner_id in intent_cell.owners(IntentKind::Build) {
            build_cells_by_swarm.entry(owner_id).or_default().push(cell);
        }
    }
    for (swarm_entity, swarm_id) in &swarms {
        let has_pending_facility = planned_facilities.iter().any(|(planned, owner)| {
            planned.kind == PlannedKind::ProductionFacility && owner.0 == swarm_entity
        });
        if has_pending_facility {
            pressure.set_ticks(*swarm_id, 0);
            continue;
        }
        let mut counts = count_swarm_nanobots_by_type(*swarm_id, &nanobots);
        let mut all_busy = true;
        for (_, facility, _, _) in facilities.iter().filter(|(_, _, owner, condition)| {
            owner.0 == swarm_entity && condition.is_none_or(SupportCondition::is_operational)
        }) {
            if let Some(kind) = facility.current_target {
                *counts.entry(kind).or_default() += 1;
            }
            all_busy &= facility.is_busy(tuning.production.ticks_per_bot);
        }
        let target = population_demand.most_underfilled_type(*swarm_id, &counts);
        let pressure_continues = target.is_some() && all_busy;
        let pressure_ticks = next_production_pressure_ticks(
            pressure.ticks_for(*swarm_id),
            pressure_continues,
            tuning.production.pressure_ticks,
        );
        pressure.set_ticks(*swarm_id, pressure_ticks);
        if pressure_ticks < tuning.production.pressure_ticks {
            continue;
        }
        if target.is_none() {
            continue;
        }
        // Build-Zone constrained placement. The swarm must
        // own at least one free Build cell. Without it,
        // the swarm cannot plan a new facility and the
        // system is a no-op for this swarm this tick.
        let Some((build_cell, placement_pos)) =
            build_cells_by_swarm.get(swarm_id).and_then(|cells| {
                crate::nanobot::placement::find_build_zone_placement_accepting(
                    cells,
                    &obstacles,
                    PlannedKind::ProductionFacility.footprint_half_extent(),
                    27,
                    |position| {
                        access.accepts(
                            &access_layout,
                            &grid,
                            *swarm_id,
                            PlannedKind::ProductionFacility,
                            position,
                        )
                    },
                )
            })
        else {
            continue;
        };
        access_layout.reserve(
            *swarm_id,
            PlannedKind::ProductionFacility.footprint_transform(placement_pos.extend(0.0)),
        );
        commands.spawn((
            PlannedStructure::new(PlannedKind::ProductionFacility, build_cell)
                .with_work_budget(tuning.construction.work_ticks),
            OwnerSwarm(swarm_entity),
            planned_visual_components(
                PlannedKind::ProductionFacility,
                &structure_sprites,
                placement_pos,
            ),
        ));
        pressure.set_ticks(*swarm_id, 0);
    }
}

/// Pick the next production target for every funded idle facility. Production
/// never scans stockpiles: the input hopper
/// is the only buffer it consumes from, so the three-leg chain
/// cannot be bypassed.
///
/// Counts are scoped to the explicit owner so opponent and player populations
/// cannot leak into each other's shortage calculation.
///
/// Issue #38 / ADR-0004: counts now match the per-swarm
/// `SwarmId` rather than walking the swarm's `Children`,
/// because nanobots are top-level entities.
#[allow(clippy::type_complexity)]
pub fn production_facility_pick_target_system(
    tuning: Res<Tuning>,
    population_demand: Res<crate::nanobot::PopulationDemand>,
    nanobots: Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut facility_queries: ParamSet<(
        Query<(&ProductionFacility, &OwnerSwarm, Option<&SupportCondition>)>,
        Query<(
            &mut ProductionFacility,
            &OwnerSwarm,
            Option<&SupportCondition>,
        )>,
    )>,
    mut ledger: ResMut<ResourceLedger>,
    mut counters: Option<ResMut<BattleCounters>>,
) {
    let mut available_by_swarm = HashMap::<SwarmId, HashMap<NanobotType, u32>>::new();
    for swarm_id in &swarms {
        available_by_swarm.insert(
            *swarm_id,
            count_swarm_nanobots_by_type(*swarm_id, &nanobots),
        );
    }
    for (facility, owner, condition) in &facility_queries.p0() {
        if condition.is_some_and(|condition| !condition.is_operational()) {
            continue;
        }
        let Some(kind) = facility.current_target else {
            continue;
        };
        let Ok(owner_id) = swarms.get(owner.0).copied() else {
            continue;
        };
        *available_by_swarm
            .entry(owner_id)
            .or_insert_with(|| count_swarm_nanobots_by_type(owner_id, &nanobots))
            .entry(kind)
            .or_default() += 1;
    }

    for (mut facility, owner, condition) in &mut facility_queries.p1() {
        if condition.is_some_and(|condition| !condition.is_operational()) {
            continue;
        }
        if facility.current_target.is_some() {
            continue;
        }

        // Issue #38 / ADR-0004: the per-swarm count uses
        // the owner's `SwarmId` rather than walking
        // children, because nanobots are top-level
        // entities.
        let Ok(owner_id) = swarms.get(owner.0).copied() else {
            continue;
        };
        let counts = available_by_swarm
            .entry(owner_id)
            .or_insert_with(|| count_swarm_nanobots_by_type(owner_id, &nanobots));
        let target = population_demand.most_underfilled_type(owner_id, counts);
        // Write only on change so haul projection is not invalidated every tick.
        if facility.demand_waiting != target.is_some() {
            facility.demand_waiting = target.is_some();
        }
        let Some(kind) = target else {
            continue;
        };
        if facility.input_amount < tuning.production.cost_per_bot {
            if let Some(counters) = counters.as_deref_mut() {
                counters.record(owner_id, BattleEvent::MineralStarvedFacilityTick);
            }
            continue;
        }
        facility.input_amount -= tuning.production.cost_per_bot;
        ledger.consume_for(
            owner_id,
            facility.input_kind,
            tuning.production.cost_per_bot,
        );
        if let Some(counters) = counters.as_deref_mut() {
            counters.record(
                owner_id,
                BattleEvent::Consumed(tuning.production.cost_per_bot),
            );
        }
        facility.current_target = Some(kind);
        facility.progress = 0;
        *counts.entry(kind).or_default() += 1;
    }
}

/// Advance production and release finished output once a body-clear exterior cell
/// is free. Output retains its funded type and cycle until release or destruction.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn production_facility_work_system(
    tuning: Res<Tuning>,
    mut commands: Commands,
    mut tick: Local<u64>,
    generation: Option<Res<crate::session_lifecycle::SessionGeneration>>,
    mut previous_generation: Local<Option<u64>>,
    mut facilities: Query<(
        Entity,
        &mut ProductionFacility,
        &Transform,
        &OwnerSwarm,
        Option<&SupportCondition>,
        Option<&CombatIdentity>,
    )>,
    swarms: Query<&SwarmId, With<Swarm>>,
    nanobots: Query<&Transform, With<Nanobot>>,
    physical: crate::physical_world::PhysicalWorld,
    mut counters: Option<ResMut<BattleCounters>>,
) {
    use crate::navigation::{BODY_RADIUS, CELL_WIDTH, Obstacle};
    if let Some(generation) = generation.as_deref().map(|generation| generation.0)
        && *previous_generation != Some(generation)
    {
        *tick = 0;
        *previous_generation = Some(generation);
    }
    *tick = tick.saturating_add(1);
    let mut occupied: Vec<_> = nanobots.iter().map(|t| t.translation.truncate()).collect();
    let geometry = physical.snapshot();
    let mut ready = Vec::new();
    for (entity, mut facility, _, _, condition, identity) in &mut facilities {
        if condition.is_some_and(|c| !c.is_operational()) || facility.current_target.is_none() {
            continue;
        }
        facility.progress = facility
            .progress
            .saturating_add(1)
            .min(tuning.production.ticks_per_bot);
        if facility.progress == tuning.production.ticks_per_bot {
            let finished = *facility.finished_at.get_or_insert(*tick);
            ready.push((finished, combat_key(entity, identity), entity));
        }
    }
    ready.sort_unstable_by_key(|(finished, key, _)| (*finished, *key));
    for (_, _, entity) in ready {
        let Ok((_, mut facility, transform, owner, ..)) = facilities.get_mut(entity) else {
            continue;
        };
        let Ok(swarm_id) = swarms.get(owner.0).copied() else {
            continue;
        };
        let shape = Obstacle::structure(transform);
        let Obstacle::Rectangle { center, half } = shape else {
            unreachable!()
        };
        let min = ((center - half) / CELL_WIDTH).floor().as_ivec2() - IVec2::ONE;
        let max = ((center + half) / CELL_WIDTH).ceil().as_ivec2();
        let mut exit = None;
        'cells: for y in min.y..=max.y {
            for x in min.x..=max.x {
                let position = (IVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * CELL_WIDTH;
                if !shape.admits_body(position) {
                    continue;
                }
                if geometry.can_occupy(position)
                    && occupied.iter().all(|other| {
                        position.distance_squared(*other) >= (2.0 * BODY_RADIUS).powi(2)
                    })
                {
                    exit = Some(position);
                    break 'cells;
                }
            }
        }
        let Some(position) = exit else {
            continue;
        };
        let target = facility
            .current_target
            .take()
            .expect("ready output retains its type");
        commands.spawn((
            NanobotBundle {
                nanobot: Nanobot {},
                nanobot_type: target,
                velocity: VelocityComponent::default(),
                ai_state: AiStateComponent::new(),
                health: Health::with_tuning(&tuning),
                swarm_member: SwarmMember::new(swarm_id),
            },
            Commitment::Idle,
            Transform::from_translation(position.extend(0.0)),
        ));
        if let Some(counters) = counters.as_deref_mut() {
            counters.record(swarm_id, BattleEvent::Birth);
        }
        occupied.push(position);
        facility.progress = 0;
        facility.finished_at = None;
    }
}

pub(crate) fn reconcile_production_tuning(world: &mut World, old: &Tuning, new: &Tuning) {
    let mut facilities = world.query::<&mut ProductionFacility>();
    for mut facility in facilities.iter_mut(world) {
        if old.production.input_capacity != new.production.input_capacity {
            facility.input_capacity = new.production.input_capacity;
        }
        if old.production.ticks_per_bot != new.production.ticks_per_bot {
            facility.progress = if facility.finished_at.is_some()
                || facility.progress >= old.production.ticks_per_bot
            {
                new.production.ticks_per_bot
            } else {
                crate::tuning::rescale_ticks(
                    facility.progress,
                    old.production.ticks_per_bot,
                    new.production.ticks_per_bot,
                )
                .min(new.production.ticks_per_bot.saturating_sub(1))
            };
        }
    }
    if old.production.pressure_ticks != new.production.pressure_ticks
        && let Some(mut pressure) = world.get_resource_mut::<ProductionPressure>()
    {
        for ticks in pressure.ticks_by_swarm.values_mut() {
            *ticks = if *ticks >= old.production.pressure_ticks {
                new.production.pressure_ticks
            } else {
                crate::tuning::rescale_ticks(
                    *ticks,
                    old.production.pressure_ticks,
                    new.production.pressure_ticks,
                )
                .min(new.production.pressure_ticks.saturating_sub(1))
            };
        }
    }
}

/// Plugin that wires the production systems into the Update
/// schedule. The chain runs after `move_velocity_system` so the
/// movement step has settled before production picks targets and
/// spawns new nanobots. Auto-creation runs last in its own
/// internal chain so it sees the post-pick / post-work state of
/// the swarm and only spawns a new facility when the existing
/// ones are all busy.
///
/// Cross-plugin ordering: the auto-creation system runs
/// `before(sink_stockpile_demand_system)` so the production
/// demand layer claims a Build cell *before* the
/// sink-stockpile demand layer fills every Build cell with
/// a Sink Stockpile plan. Without this ordering, a swarm
/// with high unmet demand and a single Build cell would
/// never plan a Production Facility -- the Sink Stockpile
/// demand layer would claim the only cell first. The
/// production facility now gets first pick of any free
/// Build cell; the sink-stockpile demand layer then fills
/// the remaining cells, matching the "logistics follow
/// production" build order in the PRD.
pub struct ProductionPlugin;

impl Plugin for ProductionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tuning>()
            .init_resource::<ProductionPressure>()
            .init_resource::<FacilityClaimTimes>()
            .add_systems(
                FixedUpdate,
                (facility_lead_time_system, facility_delivery_window_system)
                    .chain()
                    .after(crate::nanobot::RegionalAllocationSet::Invalidate)
                    .before(crate::nanobot::RegionalAllocationSet::Acquire)
                    .run_if(crate::nanobot::allocation::allocation_tick_due),
            )
            .add_systems(
                FixedUpdate,
                (
                    production_facility_pick_target_system,
                    production_facility_work_system,
                    production_facility_auto_creation_system
                        .before(crate::nanobot::planned::sink_stockpile_demand_system),
                )
                    .chain()
                    .after(crate::nanobot::NanobotSimulationSet::Movement),
            );
    }
}

/// Weight of each new delivery in a facility's measured lead time.
const LEAD_TIME_SMOOTHING: f32 = 0.2;

/// Fixed ticks for one delivery from a pickup `pickup_distance` away, before
/// any delivery has been measured: the trip to the pickup, loading a full
/// load, and the trip back.
pub fn estimated_lead_ticks(tuning: &Tuning, pickup_distance: f32) -> f32 {
    let carry = tuning.logistics.hauler_carry_capacity.max(1);
    let loading = (carry * tuning.logistics.material_work_ticks) as f32;
    let travel = 2.0 * pickup_distance.max(0.0) / tuning.logistics.bot_speed;
    loading + travel
}

/// Minerals a facility consumes during `lead_ticks`, rounded up to whole
/// Hauler loads: it consumes one cycle's cost per production cycle while a
/// delivery is under way (Little's law).
pub fn window_for_lead(tuning: &Tuning, lead_ticks: f32) -> u32 {
    let carry = tuning.logistics.hauler_carry_capacity.max(1);
    let per_tick =
        tuning.production.cost_per_bot as f32 / tuning.production.ticks_per_bot.max(1) as f32;
    let loads = (per_tick * lead_ticks.max(0.0) / carry as f32).ceil() as u32;
    loads.saturating_mul(carry)
}

/// Delivery window for an unmeasured facility whose nearest stocked pickup is
/// `pickup_distance` away.
pub fn delivery_window(tuning: &Tuning, pickup_distance: f32) -> u32 {
    window_for_lead(tuning, estimated_lead_ticks(tuning, pickup_distance))
}

/// Facility claims Haulers hold, by Hauler, with the allocation tick each was
/// first seen; kept until the claim ends.
#[derive(Debug, Default, Resource)]
pub struct FacilityClaimTimes {
    claims: HashMap<Entity, ClaimTime>,
}

#[derive(Debug, Clone, Copy)]
struct ClaimTime {
    facility: Entity,
    claimed_at: u64,
    arrived: bool,
}

/// Time each Hauler's facility claim from when it is first seen until the
/// Hauler stands at the facility with its load, and fold it into the
/// facility's measured lead time. Time spent waiting for hopper room after
/// arriving is excluded, so a facility that cannot consume faster does not
/// read as far away. Runs on the allocation cadence, when claims are made.
#[allow(clippy::type_complexity)]
pub fn facility_lead_time_system(
    clock: Res<crate::nanobot::AllocationClock>,
    mut times: ResMut<FacilityClaimTimes>,
    haulers: Query<
        (
            Entity,
            &Transform,
            &crate::nanobot::LogisticsReservation,
            Option<&crate::nanobot::Cargo>,
            Has<crate::nanobot::DirectMovementComponent>,
        ),
        With<crate::nanobot::HaulerAssignment>,
    >,
    mut facilities: Query<(
        &mut ProductionFacility,
        &Transform,
        Option<&SupportCondition>,
    )>,
) {
    let now = clock.tick();
    let fixed_ticks_per_period =
        crate::nanobot::ALLOCATION_TICK_PERIOD.as_secs_f32() * crate::SIMULATION_HZ as f32;
    let mut live = std::collections::HashSet::new();
    for (hauler, transform, reservation, cargo, moving) in &haulers {
        let Ok((mut facility, facility_transform, condition)) =
            facilities.get_mut(reservation.destination)
        else {
            continue;
        };
        if condition.is_some_and(|condition| !condition.is_operational()) {
            continue;
        }
        live.insert(hauler);
        let fresh = ClaimTime {
            facility: reservation.destination,
            claimed_at: now,
            arrived: false,
        };
        let time = times.claims.entry(hauler).or_insert(fresh);
        if time.facility != reservation.destination {
            *time = fresh;
        }
        let loaded = cargo.is_some_and(|cargo| cargo.amount > 0);
        if time.arrived
            || moving
            || !loaded
            || !crate::nanobot::InteractionRegion::structure(facility_transform)
                .contains(transform.translation.truncate())
        {
            continue;
        }
        time.arrived = true;
        // Claims are made after this system runs, so each was first seen one
        // period after it was made.
        let lead = (now - time.claimed_at + 1) as f32 * fixed_ticks_per_period;
        facility.measured_lead_ticks = Some(facility.measured_lead_ticks.map_or(lead, |average| {
            average + LEAD_TIME_SMOOTHING * (lead - average)
        }));
    }
    times.claims.retain(|hauler, _| live.contains(hauler));
}

/// Size each busy facility's delivery window from its measured lead time, or
/// before any delivery is measured, from its nearest stocked Source or Sink
/// Stockpile of the same owner. Runs on the allocation cadence, since only
/// claims read it. The window never shrinks below what Haulers already
/// claimed, so a pickup emptying or restocking nearby cannot revoke a delivery
/// in flight; Hauler
/// claims are summed only when some window would shrink.
#[allow(clippy::type_complexity)]
pub fn facility_delivery_window_system(
    tuning: Res<Tuning>,
    mut facilities: Query<(
        Entity,
        &mut ProductionFacility,
        &Transform,
        &OwnerSwarm,
        Option<&SupportCondition>,
    )>,
    pickups: Query<(
        &Stockpile,
        &Transform,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    reservations: Query<
        &crate::nanobot::LogisticsReservation,
        With<crate::nanobot::HaulerAssignment>,
    >,
) {
    let mut claimed: Option<HashMap<Entity, u32>> = None;
    for (entity, mut facility, transform, owner, condition) in &mut facilities {
        if condition.is_some_and(|condition| !condition.is_operational())
            || (facility.current_target.is_none() && !facility.demand_waiting)
        {
            continue;
        }
        let position = transform.translation.truncate();
        let nearest = pickups
            .iter()
            .filter(|(stockpile, _, pickup_owner, condition)| {
                stockpile.amount > 0
                    && pickup_owner.is_some_and(|pickup_owner| pickup_owner.0 == owner.0)
                    && condition.is_none_or(|condition| condition.is_operational())
            })
            .map(|(_, pickup, ..)| pickup.translation.truncate().distance(position))
            .min_by(f32::total_cmp);
        let derived = facility
            .measured_lead_ticks
            .or_else(|| nearest.map(|distance| estimated_lead_ticks(&tuning, distance)))
            .map_or(0, |lead| window_for_lead(&tuning, lead));
        let window = if derived >= facility.queued_space {
            derived
        } else {
            let claimed = claimed.get_or_insert_with(|| {
                let mut claimed = HashMap::<Entity, u32>::new();
                for reservation in &reservations {
                    *claimed.entry(reservation.destination).or_default() +=
                        reservation.destination_remaining;
                }
                claimed
            });
            let held = (facility.input_amount + claimed.get(&entity).copied().unwrap_or_default())
                .saturating_sub(facility.input_capacity);
            derived.max(held)
        };
        if facility.queued_space != window {
            facility.queued_space = window;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. End-to-end behaviour lives in the production
    //! facility behavior tests.

    use super::*;

    #[test]
    fn production_pressure_requires_consecutive_ticks() {
        let mut ticks = 0;
        for _ in 0..Tuning::default().production.pressure_ticks - 1 {
            ticks = next_production_pressure_ticks(
                ticks,
                true,
                Tuning::default().production.pressure_ticks,
            );
        }
        assert_eq!(ticks, Tuning::default().production.pressure_ticks - 1);
        assert_eq!(
            next_production_pressure_ticks(
                ticks,
                false,
                Tuning::default().production.pressure_ticks
            ),
            0
        );
        assert_eq!(
            next_production_pressure_ticks(
                Tuning::default().production.pressure_ticks,
                true,
                Tuning::default().production.pressure_ticks
            ),
            Tuning::default().production.pressure_ticks,
        );
    }

    #[test]
    fn production_facility_starts_idle() {
        let f = ProductionFacility::new();
        assert!(!f.is_busy(Tuning::default().production.ticks_per_bot));
        assert_eq!(f.progress, 0);
        assert_eq!(f.current_target, None);
    }

    #[test]
    fn claimable_capacity_queues_space_only_when_a_cycle_will_start() {
        let tuning = Tuning::default();
        let queued = tuning.production.cost_per_bot;
        let mut f = ProductionFacility::new();
        assert_eq!(f.claimable_capacity(&tuning), f.input_capacity);
        f.demand_waiting = true;
        assert_eq!(f.claimable_capacity(&tuning), f.input_capacity + queued);
        f.demand_waiting = false;
        f.current_target = Some(NanobotType::Worker);
        assert_eq!(f.claimable_capacity(&tuning), f.input_capacity + queued);
    }

    fn balance_tuning() -> Tuning {
        let mut tuning = Tuning::default();
        tuning.production.cost_per_bot = 5;
        tuning.production.ticks_per_bot = 120;
        tuning.logistics.hauler_carry_capacity = 10;
        tuning.logistics.material_work_ticks = 24;
        tuning.logistics.bot_speed = 5.0;
        tuning
    }

    #[test]
    fn delivery_window_covers_consumption_during_the_lead_time() {
        let tuning = balance_tuning();
        // 240 ticks loading + 240 ticks travel at 5/120 per tick = 20.
        assert_eq!(delivery_window(&tuning, 600.0), 20);
        // 240 + 600 ticks = 35 minerals, rounded up to four loads.
        assert_eq!(delivery_window(&tuning, 1_500.0), 40);
        // Loading alone takes 240 ticks: ten minerals, one load.
        assert_eq!(delivery_window(&tuning, 0.0), 10);
    }

    #[test]
    fn claimable_space_queues_at_least_one_cycle() {
        let tuning = balance_tuning();
        let mut f = ProductionFacility::with_tuning(&tuning);
        f.current_target = Some(NanobotType::Worker);
        assert_eq!(f.claimable_capacity(&tuning), f.input_capacity + 5);
        f.queued_space = 30;
        assert_eq!(f.claimable_capacity(&tuning), f.input_capacity + 30);
    }

    #[test]
    fn production_facility_is_busy_with_target() {
        let mut f = ProductionFacility::new();
        f.current_target = Some(NanobotType::Worker);
        assert!(f.is_busy(Tuning::default().production.ticks_per_bot));
    }

    #[test]
    fn production_progress_percent_reports_zero_when_idle() {
        let f = ProductionFacility::new();
        assert_eq!(
            production_progress_percent(&f, Tuning::default().production.ticks_per_bot),
            0
        );
    }

    #[test]
    fn production_progress_percent_scales_with_progress() {
        let mut f = ProductionFacility::new();
        f.current_target = Some(NanobotType::Worker);
        f.progress = 0;
        assert_eq!(
            production_progress_percent(&f, Tuning::default().production.ticks_per_bot),
            0
        );
        f.progress = Tuning::default().production.ticks_per_bot * 2 / 5;
        assert_eq!(
            production_progress_percent(&f, Tuning::default().production.ticks_per_bot),
            40
        );
        f.progress = Tuning::default().production.ticks_per_bot;
        assert_eq!(
            production_progress_percent(&f, Tuning::default().production.ticks_per_bot),
            100
        );
        // Defensive: progress over the budget must not
        // report >100%.
        f.progress = Tuning::default().production.ticks_per_bot + 5;
        assert_eq!(
            production_progress_percent(&f, Tuning::default().production.ticks_per_bot),
            100
        );
    }
}
