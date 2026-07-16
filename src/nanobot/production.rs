//! Production facilities and Production Priority control.
//!
//! Production facilities consume delivered resources and use Production
//! Priority to order typed workload shortages. Additional facilities emerge
//! from demand pressure when existing capacity is too busy. Blocked types are
//! skipped temporarily instead of stalling all production.
//!
//! ## State machine
//!
//! Each [`ProductionFacility`] cycles through:
//!
//! ```text
//!   Idle (no current_target)
//!      -> pick the highest-priority typed shortage
//!      -> try consume material from nearest stockpile
//!      -> on success: Working (current_target set, progress=0)
//!      -> on failure: type added to blocked set, try next
//!   Working
//!      -> advance progress each tick
//!      -> on progress >= PRODUCTION_TICKS_PER_BOT:
//!         spawn a new nanobot of current_target
//!         reset to Idle, clear blocked_types
//! ```
//!
//! Material flow: the facility pulls the full
//! [`PRODUCTION_COST_PER_BOT`] from the nearest local stockpile at
//! the start of a production cycle, so the resource is consumed
//! up-front rather than in dribbles. This matches the build
//! system's per-tick consumption pattern but at cycle boundaries.
//! No teleporting resources; the hauler chain is the upstream
//! source of those stockpiles.
//!
//! Shared cost/time: all three early types (Worker, Hauler,
//! Defender) cost the same number of minerals and take the same
//! number of ticks to produce. Differentiated costs are a
//! follow-up issue per the PRD.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::ai::AiStateComponent;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{Commitment, NanobotType};
use crate::nanobot::components::{Health, Nanobot, Swarm, SwarmId, SwarmMember, VelocityComponent};
use crate::nanobot::gather::world_to_cell;
use crate::nanobot::maintenance::SupportCondition;
use crate::nanobot::placement::{find_build_zone_placement, scaled_building_footprint_radius};
use crate::nanobot::planned::{
    PlannedKind, PlannedProductionTarget, PlannedStructure, planned_visual_components,
};
use crate::nanobot::{NanobotBundle, NanobotSprites};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};
use crate::structure_sprites::StructureSprites;

/// Material (in `ResourceKind::Minerals`) consumed to produce one
/// nanobot. Shared across all three early types per the project's
/// "shared cost/time" decision. The facility takes the full cost
/// up-front at the start of a production cycle.
pub const PRODUCTION_COST_PER_BOT: u32 = 20;

/// Number of ticks a facility needs to finish a production cycle
/// after consuming material. Shared across all three early types.
/// At the runtime fixed-update frequency, 120 ticks is two seconds.
pub const PRODUCTION_TICKS_PER_BOT: u32 = 120;

/// Capacity of a [`ProductionFacility`]'s own input hopper. Haulers
/// (logistics leg 3) deliver minerals into this buffer; production
/// consumes exclusively from it. Sized to hold two production cycles
/// so a facility can buffer short delivery gaps without hoarding at
/// stockpile scale.
pub const PRODUCTION_INPUT_CAPACITY: u32 = 40;

/// Priority-share deficit threshold for the legacy no-`PopulationDemand`
/// facility-emergence fallback.
pub const FACILITY_EMERGE_DEFICIT_THRESHOLD: i32 = 5;

/// Consecutive fixed ticks of Production Pressure required before committing
/// another Production Facility.
pub const PRODUCTION_PRESSURE_TICKS: u32 = 60;

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

fn next_production_pressure_ticks(current: u32, pressure_continues: bool) -> u32 {
    if pressure_continues {
        current.saturating_add(1).min(PRODUCTION_PRESSURE_TICKS)
    } else {
        0
    }
}

/// Player-set production priority. Inserted as a Bevy
/// [`Resource`] so the production systems can read and write it
/// without a public crate API surface.
///
/// The values are relative weights used to order typed workload shortages.
/// Zero is the lowest priority, not a production ban: required work is still
/// filled after positively weighted shortages.
#[derive(Debug, Clone, Resource)]
pub struct ProductionPriority {
    pub weights: HashMap<NanobotType, u32>,
}

impl ProductionPriority {
    /// Empty priority. Tests use this to set only the types they
    /// care about; the game starts with [`Default`].
    pub fn new() -> Self {
        Self {
            weights: HashMap::new(),
        }
    }

    /// Set the priority weight for `kind` without clamping.
    pub fn set_weight(&mut self, kind: NanobotType, weight: u32) {
        self.weights.insert(kind, weight);
    }

    /// Weight for `kind`, or `0` when unset.
    pub fn weight(&self, kind: NanobotType) -> u32 {
        self.weights.get(&kind).copied().unwrap_or(0)
    }

    /// Sum of all weights.
    pub fn total_weight(&self) -> u32 {
        self.weights.values().sum()
    }

    /// Fraction of the total priority allocated to `kind`, in
    /// `[0.0, 1.0]`. Returns `0.0` when the total weight is
    /// zero so callers can treat "unset" and "explicitly zero"
    /// identically.
    pub fn normalized_weight(&self, kind: NanobotType) -> f32 {
        let total = self.total_weight();
        if total == 0 {
            return 0.0;
        }
        self.weight(kind) as f32 / total as f32
    }

    /// Integer percent (0-100) of the total priority allocated
    /// to `kind`, rounded to the nearest whole number. The
    /// UI uses this for the percentage labels; the production
    /// picker compares raw weights.
    pub fn percentage(&self, kind: NanobotType) -> u32 {
        (self.normalized_weight(kind) * 100.0).round() as u32
    }

    /// Change `kind`'s weight by `delta`, saturating at zero on
    /// the low end. Returns the new weight, or `None` when the
    /// change would zero the total. The slider UI uses this so
    /// the player cannot drag every type to zero.
    pub fn try_change_weight(&mut self, kind: NanobotType, delta: i32) -> Option<u32> {
        let current = self.weight(kind) as i32;
        let proposed = current.saturating_add(delta).max(0) as u32;
        let new_total = self
            .total_weight()
            .saturating_sub(current as u32)
            .saturating_add(proposed);
        if new_total == 0 {
            return None;
        }
        self.weights.insert(kind, proposed);
        Some(proposed)
    }
}

impl Default for ProductionPriority {
    fn default() -> Self {
        let mut priority = Self::new();
        priority.set_weight(NanobotType::Worker, 6);
        priority.set_weight(NanobotType::Hauler, 3);
        priority.set_weight(NanobotType::Defender, 1);
        priority
    }
}

/// Marker for a [`crate::nanobot::Swarm`] that is driven by
/// prepainted intent and a fixed production priority (the project
/// glossary's "Opponent Swarm"). Opponent nanobots still run
/// through the same scoring, logistics, and production systems
/// as the player swarm; the marker only lets callers query
/// opponents separately.
#[derive(Debug, Component, Default)]
pub struct OpponentSwarm {}

/// Per-swarm production priority override. Attached to a
/// [`crate::nanobot::Swarm`] to give it its own production ordering.
/// When present, the production systems prefer this over the
/// global [`ProductionPriority`] resource, so the opponent can
/// keep fixed priorities while the player keeps mutating the global
/// resource.
#[derive(Debug, Component, Clone)]
pub struct SwarmProduction {
    pub priority: ProductionPriority,
}

impl SwarmProduction {
    pub fn new(priority: ProductionPriority) -> Self {
        Self { priority }
    }
}

/// Ties a production facility to the swarm that owns it. Used
/// to resolve the per-swarm [`SwarmProduction`] (or the
/// global [`ProductionPriority`] resource as a fallback) and to
/// decide which swarm a completed cycle spawns its new
/// nanobot under. Facilities without this marker fall back to
/// the global priority and the first swarm in the world, which
/// keeps the pre-multi-swarm tests working.
#[derive(Debug, Component, Clone, Copy)]
pub struct OwnerSwarm(pub Entity);

fn facility_belongs_to_swarm(
    owner: Option<&OwnerSwarm>,
    swarm_entity: Entity,
    swarm_id: SwarmId,
) -> bool {
    owner.is_some_and(|OwnerSwarm(owner)| *owner == swarm_entity)
        || (owner.is_none() && swarm_id == SwarmId::PLAYER)
}

/// An automatic production facility. Spawned by
/// `production_facility_auto_creation_system` near a swarm that
/// has unmet production demand. The facility's own state is
/// carried on this component so multiple facilities can run
/// independently.
#[derive(Debug, Component, Clone)]
pub struct ProductionFacility {
    /// Tick counter within the current production cycle. Reset
    /// to 0 when a new cycle starts; reaches
    /// [`PRODUCTION_TICKS_PER_BOT`] to finish the cycle.
    pub progress: u32,
    /// Type currently being produced, or `None` if the facility
    /// is idle and waiting to pick its next target.
    pub current_target: Option<NanobotType>,
    /// Types the facility could not start producing this cycle
    /// (e.g. because its input hopper is too low for the cost).
    /// The system clears this set at the end of a cycle so
    /// blocked types get re-tried in the next one.
    pub blocked_types: HashSet<NanobotType>,
    /// Resource kind the input hopper accepts. Always
    /// [`ResourceKind::Minerals`] in the first implementation;
    /// kept as a field so the hauler sink matcher can pair it
    /// against the hauler's carried kind without a hardcoded
    /// assumption.
    pub input_kind: ResourceKind,
    /// Material currently sitting in the facility's input hopper.
    /// Haulers (logistics leg 3) deliver into this buffer;
    /// production pulls [`PRODUCTION_COST_PER_BOT`] from it at
    /// the start of each cycle. This is the ONLY buffer
    /// production consumes from -- a sink stockpile no longer
    /// feeds production directly, so the three-leg chain is
    /// real and the hauler cannot be bypassed.
    pub input_amount: u32,
    /// Maximum material the input hopper can hold. A full hopper
    /// reports zero free space, so the hauler sink matcher skips
    /// it until production drains some.
    pub input_capacity: u32,
}

impl ProductionFacility {
    /// New idle facility with an empty input hopper, used by the
    /// auto-creation promotion path and by tests. A facility
    /// starts idle and stays idle until a hauler delivers enough
    /// material for at least one production cycle.
    pub fn new() -> Self {
        Self {
            progress: 0,
            current_target: None,
            blocked_types: HashSet::new(),
            input_kind: ResourceKind::Minerals,
            input_amount: 0,
            input_capacity: PRODUCTION_INPUT_CAPACITY,
        }
    }

    /// True when the facility is currently producing a nanobot.
    /// Used by the auto-creation system to detect "all existing
    /// facilities are too busy".
    pub fn is_busy(&self) -> bool {
        self.current_target.is_some()
    }

    /// True when `kind` is currently in the blocked set.
    pub fn is_blocked(&self, kind: NanobotType) -> bool {
        self.blocked_types.contains(&kind)
    }

    /// Free capacity in the input hopper for hauler delivery.
    /// Mirrors [`crate::resources::Stockpile::free_space`] and
    /// [`crate::nanobot::Charger::free_space`] so the hauler
    /// sink selection treats all three terminal/buffer kinds
    /// through the same shape.
    pub fn input_free_space(&self) -> u32 {
        self.input_capacity.saturating_sub(self.input_amount)
    }
}

impl Default for ProductionFacility {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy compatibility picker used only when [`crate::nanobot::PopulationDemand`]
/// is unavailable. It picks the type with the largest proportional priority
/// share deficit, skipping types in `blocked`. Returns `None` when every type is
/// blocked or the total priority is zero. Ties are broken by
/// [`NanobotType::ALL`] order, so the picker is deterministic.
///
/// With no current population, every current share is zero
/// and the picker returns the type with the largest target
/// share (the "start with the most-demanded type" rule).
pub fn pick_type_for_legacy_priority_share_fallback(
    targets: &ProductionPriority,
    current_counts: &HashMap<NanobotType, u32>,
    blocked: &HashSet<NanobotType>,
) -> Option<NanobotType> {
    let total_weight = targets.total_weight();
    if total_weight == 0 {
        return None;
    }
    let total_count: u32 = current_counts.values().sum();
    let mut best: Option<(f32, NanobotType)> = None;
    for &kind in &NanobotType::ALL {
        if blocked.contains(&kind) {
            continue;
        }
        let target_share = targets.weight(kind) as f32 / total_weight as f32;
        let current_share = if total_count == 0 {
            0.0
        } else {
            *current_counts.get(&kind).unwrap_or(&0) as f32 / total_count as f32
        };
        let deficit = target_share - current_share;
        match best {
            None => best = Some((deficit, kind)),
            Some((d, _)) if deficit > d => best = Some((deficit, kind)),
            _ => {}
        }
    }
    best.filter(|(d, _)| *d > 0.0).map(|(_, k)| k)
}

/// Pick the required type with the highest weighted shortage. Stable type order
/// breaks ties. A zero-weight shortage remains eligible, so priority never
/// disables required production.
pub fn pick_type_for_demand(
    priority: &ProductionPriority,
    available_counts: &HashMap<NanobotType, u32>,
    desired_counts: &HashMap<NanobotType, u32>,
    blocked: &HashSet<NanobotType>,
) -> Option<NanobotType> {
    let mut best = None;
    for kind in NanobotType::ALL {
        if blocked.contains(&kind) {
            continue;
        }
        let desired = desired_counts.get(&kind).copied().unwrap_or_default();
        let available = available_counts.get(&kind).copied().unwrap_or_default();
        let missing = desired.saturating_sub(available);
        if missing == 0 {
            continue;
        }
        let score = missing as u64 * priority.weight(kind) as u64;
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, kind));
        }
    }
    best.map(|(_, kind)| kind)
}

/// Legacy compatibility pressure metric used when
/// [`crate::nanobot::PopulationDemand`] is unavailable. It sums positive
/// priority-share deficits in percentage points. Returns `0` when no priority
/// is configured.
pub fn total_deficit(
    targets: &ProductionPriority,
    current_counts: &HashMap<NanobotType, u32>,
) -> i32 {
    let total_weight = targets.total_weight();
    if total_weight == 0 {
        return 0;
    }
    let total_count: u32 = current_counts.values().sum();
    let mut total = 0.0_f32;
    for &kind in &NanobotType::ALL {
        let target_share = targets.weight(kind) as f32 / total_weight as f32;
        let current_share = if total_count == 0 {
            0.0
        } else {
            *current_counts.get(&kind).unwrap_or(&0) as f32 / total_count as f32
        };
        let deficit = (target_share - current_share).max(0.0);
        total += deficit * 100.0;
    }
    total.round() as i32
}

/// Cycle progress for a [`ProductionFacility`] as an
/// integer percent in `[0, 100]`. An idle facility
/// (`current_target = None`) reports 0% so the label
/// formatter does not have to special-case it. A working
/// facility's percent is `progress / PRODUCTION_TICKS_PER_BOT`
/// floored to an integer; the label uses this directly.
///
/// The function is pure and lives next to the
/// production data so unit tests can pin the contract
/// without a Bevy `App`. The structure-overlay module
/// uses it through the `crate::nanobot` re-export.
pub fn production_progress_percent(facility: &ProductionFacility) -> u32 {
    if facility.current_target.is_none() {
        return 0;
    }
    if PRODUCTION_TICKS_PER_BOT == 0 {
        return 100;
    }
    let pct = (facility.progress as u64 * 100 / PRODUCTION_TICKS_PER_BOT as u64) as u32;
    pct.min(100)
}

/// Count nanobots in the world, keyed by type. Used by the
/// production systems to measure the current population mix.
///
/// The first implementation counts every nanobot with a
/// `NanobotType` component globally; later issues can scope this
/// to a specific swarm once multi-swarm production lands.
///
/// Issue #38 / ADR-0004: the query matches
/// `(&NanobotType, &SwarmMember)` so the count math is
/// consistent with the per-swarm variant; this function
/// counts all nanobots regardless of swarm, which is the
/// pre-multi-swarm fallback for unowned facilities.
pub fn count_nanobots_by_type(
    nanobots: &Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
) -> HashMap<NanobotType, u32> {
    let mut counts = HashMap::new();
    for (ty, _) in nanobots.iter() {
        *counts.entry(*ty).or_insert(0) += 1;
    }
    counts
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
/// 1. the swarm has a typed shortage selected by Production Priority, AND
/// 2. every operational facility that belongs to this swarm
///    (those with [`OwnerSwarm`] pointing at it, plus unowned
///    facilities for the legacy player fallback) stays busy for
///    [`PRODUCTION_PRESSURE_TICKS`] consecutive ticks, AND
/// 3. the swarm owns at least one `Build`-painted cell that
///    does not already host a planned or completed
///    structure. The Build Zone is the placement constraint
///    (issue #27 acceptance: "Planned Production Facility
///    placement is constrained to an owned Build Zone").
///
/// The plan carries a [`PlannedProductionTarget`] sidecar
/// recording the type the completed facility should produce
/// first, so the demand layer can pre-allocate the highest weighted shortage
/// at planning time. The plan
/// itself does NOT consume any material: build work is
/// worker-time-only in v1, so a Worker can build the plan
/// even when the swarm is short on minerals. The completed
/// `ProductionFacility` then runs through the existing pick
/// + work systems, which consume material on the first
///   pick cycle (or skip the cycle if material is unavailable,
///   matching the existing blocked-type behaviour).
///
/// When `PopulationDemand` is absent, a legacy compatibility fallback derives
/// demand from priority shares. This seam supports isolated tests and callers
/// that do not install the runtime demand resource.
///
/// Acceptance: "No new Production Facility is planned when
/// no suitable Build Zone exists." A swarm without any
/// owned Build cells cannot plan a Production Facility, so
/// the auto-creation is a no-op for that swarm. This is the
/// "Build Zone constrains placement" half of the contract.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn production_facility_auto_creation_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    structure_sprites: Res<StructureSprites>,
    global_priority: Res<ProductionPriority>,
    population_demand: Option<Res<crate::nanobot::PopulationDemand>>,
    mut pressure: ResMut<ProductionPressure>,
    nanobots: Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
    facilities: Query<(
        Entity,
        &ProductionFacility,
        Option<&OwnerSwarm>,
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
    planned_facilities: Query<(&PlannedStructure, Option<&OwnerSwarm>)>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    swarm_productions: Query<&SwarmProduction>,
    swarms: Query<(Entity, &SwarmId), With<Swarm>>,
) {
    // Build the set of cells already occupied by any
    // planned or completed structure. A planned
    // Production Facility is in this set, so subsequent
    // ticks do not pile a second plan on the same cell.
    let mut cells_with_target: HashSet<IVec2> = HashSet::new();
    let mut obstacles: Vec<(Vec2, f32)> = deposits
        .iter()
        .map(|(deposit, transform)| (transform.translation.truncate(), deposit.radius))
        .collect();
    for transform in &existing_targets {
        let pos = transform.translation.truncate();
        cells_with_target.insert(world_to_cell(pos));
        obstacles.push((pos, scaled_building_footprint_radius(transform)));
    }
    // Pre-compute the list of (cell, swarm) pairs that are
    // Build-painted, owned by a swarm, and not already
    // occupied. The swarm-by-id map and the per-cell
    // ownership lookup drive the per-swarm placement
    // decision below.
    let mut build_cells_by_swarm: HashMap<SwarmId, Vec<IVec2>> = HashMap::new();
    for (cell, intent_cell) in grid.iter_active_cells() {
        if !intent_cell.has(IntentKind::Build) {
            continue;
        }
        if cells_with_target.contains(&cell) {
            continue;
        }
        // Unowned Build paint is treated as a swarm-less
        // cell: the per-swarm loop below will not see it,
        // so the demand layer must have a swarm-owned Build
        // Zone to plan a Production Facility. The
        // per-swarm contract (issue #20) makes unowned
        // paint visible to every swarm, but the planning
        // layer here is per-swarm so we filter on the
        // painted owner only.
        let Some(owner_id) = intent_cell.owner(IntentKind::Build) else {
            continue;
        };
        build_cells_by_swarm.entry(owner_id).or_default().push(cell);
    }
    for (swarm_entity, swarm_id) in &swarms {
        let has_pending_facility = planned_facilities.iter().any(|(planned, owner)| {
            planned.kind == PlannedKind::ProductionFacility
                && owner.is_some_and(|owner| owner.0 == swarm_entity)
        });
        if has_pending_facility {
            pressure.set_ticks(*swarm_id, 0);
            continue;
        }
        let priority = swarm_productions
            .get(swarm_entity)
            .map(|sp| &sp.priority)
            .unwrap_or(&*global_priority);
        let mut counts = count_swarm_nanobots_by_type(*swarm_id, &nanobots);
        for (_, facility, owner, _) in &facilities {
            let belongs_to_swarm = facility_belongs_to_swarm(owner, swarm_entity, *swarm_id);
            if belongs_to_swarm && let Some(kind) = facility.current_target {
                *counts.entry(kind).or_default() += 1;
            }
        }
        let target = if let Some(demand) = population_demand.as_deref() {
            let desired = NanobotType::ALL
                .into_iter()
                .map(|kind| (kind, demand.desired_for(*swarm_id, kind)))
                .collect();
            pick_type_for_demand(priority, &counts, &desired, &HashSet::new())
        } else {
            // Compatibility seam for isolated tests and callers without the
            // runtime typed-demand resource.
            if total_deficit(priority, &counts) < FACILITY_EMERGE_DEFICIT_THRESHOLD {
                None
            } else {
                pick_type_for_legacy_priority_share_fallback(priority, &counts, &HashSet::new())
            }
        };
        // Unowned facilities belong to the player fallback so legacy scenarios
        // do not lend the same capacity to every visible swarm.
        let relevant: Vec<&ProductionFacility> = facilities
            .iter()
            .filter(|(_, _, owner, condition)| {
                condition.is_none_or(|condition| condition.is_operational())
                    && facility_belongs_to_swarm(*owner, swarm_entity, *swarm_id)
            })
            .map(|(_, facility, _, _)| facility)
            .collect();
        let pressure_continues =
            target.is_some() && relevant.iter().all(|facility| facility.is_busy());
        let pressure_ticks =
            next_production_pressure_ticks(pressure.ticks_for(*swarm_id), pressure_continues);
        pressure.set_ticks(*swarm_id, pressure_ticks);
        if pressure_ticks < PRODUCTION_PRESSURE_TICKS {
            continue;
        }
        let Some(target) = target else {
            continue;
        };
        // Build-Zone constrained placement. The swarm must
        // own at least one free Build cell. Without it,
        // the swarm cannot plan a new facility and the
        // system is a no-op for this swarm this tick.
        let Some((build_cell, placement_pos)) = build_cells_by_swarm
            .get(swarm_id)
            .and_then(|cells| find_build_zone_placement(cells, &obstacles, 27))
        else {
            continue;
        };
        commands.spawn((
            PlannedStructure::new(PlannedKind::ProductionFacility, build_cell),
            PlannedProductionTarget(target),
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

/// Pick the next production target for every idle facility and
/// consume the material up-front from the facility's own input
/// hopper. If the hopper does not hold a full
/// [`PRODUCTION_COST_PER_BOT`], the candidate type is added to
/// the facility's blocked set and the picker tries the next one.
///
/// A facility whose blocked set covers every type stays idle
/// (it has nothing to produce) until a hauler delivers material
/// via logistics leg 3 and a future pick run can break the
/// deadlock. Production never scans stockpiles: the input hopper
/// is the only buffer it consumes from, so the three-leg chain
/// cannot be bypassed.
///
/// Each facility reads Production Priority from the [`SwarmProduction`] of its
/// [`OwnerSwarm`] (if present) or falls back to the global
/// [`ProductionPriority`] resource. Counts are scoped to the owner so opponent
/// and player populations cannot leak into each other's shortage calculation.
///
/// Issue #38 / ADR-0004: counts now match the per-swarm
/// `SwarmId` rather than walking the swarm's `Children`,
/// because nanobots are top-level entities.
#[allow(clippy::type_complexity)]
pub fn production_facility_pick_target_system(
    global_priority: Res<ProductionPriority>,
    population_demand: Option<Res<crate::nanobot::PopulationDemand>>,
    nanobots: Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
    swarm_productions: Query<&SwarmProduction>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut facility_queries: ParamSet<(
        Query<(&ProductionFacility, Option<&OwnerSwarm>)>,
        Query<(
            &mut ProductionFacility,
            Option<&OwnerSwarm>,
            Option<&SupportCondition>,
        )>,
    )>,
    mut ledger: ResMut<ResourceLedger>,
) {
    let mut available_by_swarm = HashMap::<SwarmId, HashMap<NanobotType, u32>>::new();
    for swarm_id in &swarms {
        available_by_swarm.insert(
            *swarm_id,
            count_swarm_nanobots_by_type(*swarm_id, &nanobots),
        );
    }
    for (facility, owner) in &facility_queries.p0() {
        let Some(kind) = facility.current_target else {
            continue;
        };
        let owner_id = owner
            .and_then(|OwnerSwarm(owner)| swarms.get(*owner).ok().copied())
            .unwrap_or(SwarmId::PLAYER);
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
        if facility.is_busy() {
            continue;
        }

        // Owned facility: owner's priority and population.
        // Unowned facility: global priority and player population.
        // The unowned branch is the fallback that
        // keeps the pre-multi-swarm tests green.
        //
        // Issue #38 / ADR-0004: the per-swarm count uses
        // the owner's `SwarmId` rather than walking
        // children, because nanobots are top-level
        // entities.
        let (priority, owner_id): (&ProductionPriority, SwarmId) = match owner {
            Some(OwnerSwarm(swarm)) => {
                let priority = swarm_productions
                    .get(*swarm)
                    .map(|sp| &sp.priority)
                    .unwrap_or(&*global_priority);
                let swarm_id = swarms.get(*swarm).copied().unwrap_or(SwarmId::PLAYER);
                (priority, swarm_id)
            }
            None => (&*global_priority, SwarmId::PLAYER),
        };
        let counts = available_by_swarm
            .entry(owner_id)
            .or_insert_with(|| count_swarm_nanobots_by_type(owner_id, &nanobots));

        // Try shortages in priority order; if the input hopper does
        // not hold a full production cost, block the type and
        // try the next. We loop because blocking one type
        // changes the ranking for the next attempt. The hopper
        // is the facility's own buffer -- a sink stockpile no
        // longer feeds production directly, so leg 3 of the
        // logistics chain (hauler: sink stockpile -> facility)
        // is the only way material reaches this point.
        //
        // `picked` tracks whether a cycle started this tick.
        // When it did, the blocked set is preserved for the
        // rest of that cycle (it is the running list of types
        // the picker walked past); the work system clears it
        // on spawn. When it did NOT, the blocked set is
        // dropped here so every required type is re-tried
        // fresh next tick. Without this drop a cold-start
        // facility deadlocks: it blocks its only
        // required type on an empty hopper, the
        // blocked set is otherwise only cleared at the end
        // of a cycle, and a cycle can never start while that
        // sole type stays blocked. This is the "blocked
        // types are skipped temporarily instead of stalling
        // all production" half of the contract.
        let mut picked = false;
        loop {
            let kind = if let Some(demand) = population_demand.as_deref() {
                let desired = NanobotType::ALL
                    .into_iter()
                    .map(|kind| (kind, demand.desired_for(owner_id, kind)))
                    .collect();
                pick_type_for_demand(priority, counts, &desired, &facility.blocked_types)
            } else {
                // Compatibility seam for isolated tests and callers without
                // the runtime typed-demand resource.
                pick_type_for_legacy_priority_share_fallback(
                    priority,
                    counts,
                    &facility.blocked_types,
                )
            };
            let Some(kind) = kind else {
                break;
            };
            if facility.input_amount >= PRODUCTION_COST_PER_BOT {
                facility.input_amount -= PRODUCTION_COST_PER_BOT;
                ledger.remove_for(owner_id, facility.input_kind, PRODUCTION_COST_PER_BOT);
                facility.current_target = Some(kind);
                facility.progress = 0;
                *counts.entry(kind).or_default() += 1;
                picked = true;
                break;
            }
            facility.blocked_types.insert(kind);
        }
        if !picked {
            facility.blocked_types.clear();
        }
    }
}

/// Advance each busy facility's progress counter. When progress
/// reaches [`PRODUCTION_TICKS_PER_BOT`], spawn a new nanobot of
/// the facility's `current_target` as a child of the owning
/// [`Swarm`] (or the first swarm in the world for unowned
/// facilities, matching the pre-multi-swarm behaviour), then
/// reset the facility to idle and clear the blocked set so the
/// next cycle re-tries blocked types.
#[allow(clippy::type_complexity)]
pub fn production_facility_work_system(
    mut commands: Commands,
    mut facilities: Query<(
        &mut ProductionFacility,
        &Transform,
        Option<&OwnerSwarm>,
        Option<&SupportCondition>,
    )>,
    swarms: Query<(Entity, Option<&SwarmId>), With<Swarm>>,
    opponent_swarms: Query<(), With<OpponentSwarm>>,
    sprites: Option<Res<NanobotSprites>>,
) {
    for (mut facility, transform, owner, condition) in &mut facilities {
        if condition.is_some_and(|condition| !condition.is_operational()) {
            continue;
        }
        let Some(target) = facility.current_target else {
            continue;
        };
        facility.progress = facility.progress.saturating_add(1);
        if facility.progress < PRODUCTION_TICKS_PER_BOT {
            continue;
        }
        // Cycle complete: spawn the nanobot. The owner
        // swarm is the natural parent (the facility belongs
        // to it), with a fallback to the first swarm in the
        // world for unowned facilities. If no swarm exists
        // the spawn is dropped (tests with no swarm drive
        // the systems directly).
        //
        // Issue #38 / ADR-0004: produced nanobots are
        // top-level entities with world `Transform`s, not
        // children of the swarm. The previous
        // `local_pos = pos - swarm_pos` math ended up with
        // the bot at the right world position only when
        // nothing ever re-read the swarm's `Transform`;
        // every other system reads `transform.translation`
        // as a world coordinate, so parented bots walked to
        // `local_destination + swarm_pos` -- the cell
        // center + half-cell offset that drove the
        // "top-right corner / bottom-left structure" bug.
        // The swarm's own `Transform` is preserved here
        // only as a spawn-origin / ownership marker; the
        // bot ends up at `pos` (the facility's world
        // position) directly.
        let parent = owner
            .map(|OwnerSwarm(e)| Some(*e))
            .unwrap_or_else(|| swarms.iter().next().map(|(entity, _)| entity));
        if let Some(swarm_entity) = parent {
            let pos = transform.translation.truncate();
            let is_opponent = opponent_swarms.get(swarm_entity).is_ok();
            // Look up the parent swarm's `SwarmId` so the new
            // child carries the right ownership marker.
            // Pre-multi-swarm tests that spawn a Swarm
            // without a `SwarmId` fall back to the player id;
            // the per-swarm filter is `None == None` for the
            // default `swarm_member` value, so the legacy
            // unowned-paint tests still pass.
            let swarm_id = swarms
                .get(swarm_entity)
                .map(|(_, id)| id.copied().unwrap_or(SwarmId::PLAYER))
                .unwrap_or(SwarmId::PLAYER);
            let mut entity = commands.spawn((
                NanobotBundle {
                    nanobot: Nanobot {},
                    nanobot_type: target,
                    velocity: VelocityComponent::default(),
                    ai_state: AiStateComponent::new(),
                    health: Health::default(),
                    swarm_member: SwarmMember::new(swarm_id),
                },
                Commitment::Idle,
                Transform::from_translation(pos.extend(0.0)),
            ));
            if let Some(sprites) = sprites.as_deref() {
                entity.insert(Sprite::from_image(sprites.handle(target, is_opponent)));
            }
        }
        // Reset for the next cycle. Clearing the blocked set
        // is the "blocked types are skipped temporarily"
        // half of the contract: a type that could not be
        // produced this cycle is re-evaluated next cycle.
        facility.current_target = None;
        facility.progress = 0;
        facility.blocked_types.clear();
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
        app.init_resource::<ProductionPressure>().add_systems(
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

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. End-to-end behaviour lives in the production
    //! facility and Production Priority behavior tests.

    use super::*;

    #[test]
    fn production_cycle_uses_agreed_fixed_tick_duration() {
        assert_eq!(PRODUCTION_TICKS_PER_BOT, 120);
    }

    #[test]
    fn production_pressure_requires_consecutive_ticks() {
        let mut ticks = 0;
        for _ in 0..PRODUCTION_PRESSURE_TICKS - 1 {
            ticks = next_production_pressure_ticks(ticks, true);
        }
        assert_eq!(ticks, PRODUCTION_PRESSURE_TICKS - 1);
        assert_eq!(next_production_pressure_ticks(ticks, false), 0);
        assert_eq!(
            next_production_pressure_ticks(PRODUCTION_PRESSURE_TICKS, true),
            PRODUCTION_PRESSURE_TICKS,
        );
    }

    #[test]
    fn typed_demand_uses_weighted_shortage() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 25);
        priority.set_weight(NanobotType::Hauler, 60);
        priority.set_weight(NanobotType::Defender, 15);
        let available = HashMap::from([
            (NanobotType::Worker, 2),
            (NanobotType::Hauler, 8),
            (NanobotType::Defender, 0),
        ]);
        let desired = HashMap::from([
            (NanobotType::Worker, 4),
            (NanobotType::Hauler, 9),
            (NanobotType::Defender, 3),
        ]);

        assert_eq!(
            pick_type_for_demand(&priority, &available, &desired, &HashSet::new()),
            Some(NanobotType::Hauler),
        );
    }

    #[test]
    fn excess_haulers_do_not_satisfy_defender_demand() {
        let priority = ProductionPriority::default();
        let available = HashMap::from([
            (NanobotType::Worker, 4),
            (NanobotType::Hauler, 10),
            (NanobotType::Defender, 0),
        ]);
        let desired = HashMap::from([
            (NanobotType::Worker, 1),
            (NanobotType::Hauler, 2),
            (NanobotType::Defender, 1),
        ]);

        assert_eq!(
            pick_type_for_demand(&priority, &available, &desired, &HashSet::new()),
            Some(NanobotType::Defender),
        );
    }

    #[test]
    fn priority_does_not_create_work_without_typed_demand() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 1);
        priority.set_weight(NanobotType::Defender, 99);
        let desired = HashMap::from([(NanobotType::Worker, 1)]);

        assert_eq!(
            pick_type_for_demand(&priority, &HashMap::new(), &desired, &HashSet::new()),
            Some(NanobotType::Worker),
        );
    }

    #[test]
    fn zero_priority_required_type_remains_eligible() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 100);
        priority.set_weight(NanobotType::Defender, 0);
        let available = HashMap::from([(NanobotType::Worker, 1)]);
        let desired = HashMap::from([(NanobotType::Worker, 1), (NanobotType::Defender, 1)]);

        assert_eq!(
            pick_type_for_demand(&priority, &available, &desired, &HashSet::new()),
            Some(NanobotType::Defender),
        );
    }

    #[test]
    fn in_flight_count_satisfies_one_bot_shortage() {
        let priority = ProductionPriority::default();
        let available = HashMap::from([(NanobotType::Defender, 1)]);
        let desired = HashMap::from([(NanobotType::Defender, 1)]);

        assert_eq!(
            pick_type_for_demand(&priority, &available, &desired, &HashSet::new()),
            None,
        );
    }

    #[test]
    fn production_priority_set_and_get_round_trip() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
        priority.set_weight(NanobotType::Defender, 1);
        assert_eq!(priority.weight(NanobotType::Worker), 5);
        assert_eq!(priority.weight(NanobotType::Hauler), 2);
        assert_eq!(priority.weight(NanobotType::Defender), 1);
        assert_eq!(priority.total_weight(), 8);
    }

    #[test]
    fn production_priority_unset_returns_zero() {
        // Unset weights do not contribute to shortage ordering.
        let priority = ProductionPriority::new();
        assert_eq!(priority.weight(NanobotType::Worker), 0);
        assert_eq!(priority.total_weight(), 0);
    }

    #[test]
    fn production_priority_default_seeds_all_three_types() {
        let priority = ProductionPriority::default();
        assert!(priority.weight(NanobotType::Worker) > 0);
        assert!(priority.weight(NanobotType::Hauler) > 0);
        assert!(priority.weight(NanobotType::Defender) > 0);
    }

    #[test]
    fn legacy_no_population_demand_fallback_picks_largest_priority_share_gap() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 10);
        priority.set_weight(NanobotType::Defender, 5);
        let counts = HashMap::from([
            (NanobotType::Worker, 5),
            (NanobotType::Hauler, 0),
            (NanobotType::Defender, 1),
        ]);
        assert_eq!(
            pick_type_for_legacy_priority_share_fallback(&priority, &counts, &HashSet::new(),),
            Some(NanobotType::Hauler)
        );
    }

    #[test]
    fn legacy_no_population_demand_fallback_is_stable_and_skips_blocked_types() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 5);
        let counts = HashMap::new();
        let mut blocked = HashSet::new();
        blocked.insert(NanobotType::Worker);
        assert_eq!(
            pick_type_for_legacy_priority_share_fallback(&priority, &counts, &blocked),
            Some(NanobotType::Hauler)
        );
    }

    #[test]
    fn legacy_no_population_demand_fallback_reports_no_pressure_when_balanced() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 3);
        let counts = HashMap::from([(NanobotType::Worker, 5), (NanobotType::Hauler, 3)]);
        assert_eq!(total_deficit(&priority, &counts), 0);
    }

    #[test]
    fn production_facility_starts_idle() {
        let f = ProductionFacility::new();
        assert!(!f.is_busy());
        assert_eq!(f.progress, 0);
        assert_eq!(f.current_target, None);
        assert!(f.blocked_types.is_empty());
    }

    #[test]
    fn production_facility_is_busy_with_target() {
        let mut f = ProductionFacility::new();
        f.current_target = Some(NanobotType::Worker);
        assert!(f.is_busy());
    }

    #[test]
    fn production_facility_blocked_set_tracks_types() {
        let mut f = ProductionFacility::new();
        f.blocked_types.insert(NanobotType::Hauler);
        assert!(f.is_blocked(NanobotType::Hauler));
        assert!(!f.is_blocked(NanobotType::Worker));
    }

    #[test]
    fn swarm_production_wraps_a_production_priority() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Hauler, 4);
        let swarm_production = SwarmProduction::new(priority);
        assert_eq!(swarm_production.priority.weight(NanobotType::Hauler), 4);
    }

    #[test]
    fn owner_swarm_stores_the_entity_reference() {
        let mut world = World::new();
        let swarm = world.spawn_empty().id();
        let owner = OwnerSwarm(swarm);
        assert_eq!(owner.0, swarm);
    }

    #[test]
    fn unowned_facility_belongs_only_to_player_fallback() {
        let mut world = World::new();
        let player = world.spawn_empty().id();
        let opponent = world.spawn_empty().id();

        assert!(facility_belongs_to_swarm(None, player, SwarmId::PLAYER));
        assert!(!facility_belongs_to_swarm(None, opponent, SwarmId(2),));
    }

    #[test]
    fn production_progress_percent_reports_zero_when_idle() {
        let f = ProductionFacility::new();
        assert_eq!(production_progress_percent(&f), 0);
    }

    #[test]
    fn production_progress_percent_scales_with_progress() {
        let mut f = ProductionFacility::new();
        f.current_target = Some(NanobotType::Worker);
        f.progress = 0;
        assert_eq!(production_progress_percent(&f), 0);
        f.progress = PRODUCTION_TICKS_PER_BOT * 2 / 5;
        assert_eq!(production_progress_percent(&f), 40);
        f.progress = PRODUCTION_TICKS_PER_BOT;
        assert_eq!(production_progress_percent(&f), 100);
        // Defensive: progress over the budget must not
        // report >100%.
        f.progress = PRODUCTION_TICKS_PER_BOT + 5;
        assert_eq!(production_progress_percent(&f), 100);
    }

    // ---- Production Priority weights and adjustment bounds ----

    #[test]
    fn production_priority_default_seeds_60_30_10_normalized_weights() {
        let priority = ProductionPriority::default();
        assert_eq!(priority.normalized_weight(NanobotType::Worker), 0.60);
        assert_eq!(priority.normalized_weight(NanobotType::Hauler), 0.30);
        assert_eq!(priority.normalized_weight(NanobotType::Defender), 0.10);
    }

    #[test]
    fn production_priority_normalized_weight_is_zero_when_total_is_zero() {
        // Avoids NaN from divide-by-zero.
        let priority = ProductionPriority::new();
        assert_eq!(priority.normalized_weight(NanobotType::Worker), 0.0);
        assert_eq!(priority.normalized_weight(NanobotType::Hauler), 0.0);
        assert_eq!(priority.normalized_weight(NanobotType::Defender), 0.0);
    }

    #[test]
    fn production_priority_normalized_weight_matches_weight_fraction() {
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Worker, 7);
        priority.set_weight(NanobotType::Hauler, 3);
        assert!((priority.normalized_weight(NanobotType::Worker) - 0.7).abs() < 1e-6);
        assert!((priority.normalized_weight(NanobotType::Hauler) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn production_priority_try_change_weight_applies_positive_delta() {
        let mut priority = ProductionPriority::default();
        let new = priority.try_change_weight(NanobotType::Worker, 5);
        assert_eq!(new, Some(11));
        assert_eq!(priority.weight(NanobotType::Worker), 11);
    }

    #[test]
    fn production_priority_try_change_weight_applies_negative_delta() {
        let mut priority = ProductionPriority::default();
        let new = priority.try_change_weight(NanobotType::Hauler, -3);
        assert_eq!(new, Some(0));
        assert_eq!(priority.weight(NanobotType::Hauler), 0);
    }

    #[test]
    fn production_priority_try_change_weight_clamps_to_zero() {
        // Negative delta larger than current must saturate
        // at 0, never underflow. The "total cannot become
        // zero" rule is checked in the next test.
        let mut priority = ProductionPriority::default();
        let new = priority.try_change_weight(NanobotType::Defender, -100);
        assert_eq!(new, Some(0));
        assert_eq!(priority.weight(NanobotType::Defender), 0);
    }

    #[test]
    fn production_priority_try_change_weight_rejects_zero_total() {
        // Acceptance: "the last nonzero type is clamped to
        // a nonzero value." With only Defender set, dropping
        // it to 0 would zero the total -- rejected, value
        // stays at 1.
        let mut priority = ProductionPriority::new();
        priority.set_weight(NanobotType::Defender, 1);
        let new = priority.try_change_weight(NanobotType::Defender, -1);
        assert_eq!(new, None);
        assert_eq!(priority.weight(NanobotType::Defender), 1);
    }

    #[test]
    fn production_priority_try_change_weight_allows_zero_when_other_types_remain() {
        let mut priority = ProductionPriority::default();
        let new = priority.try_change_weight(NanobotType::Defender, -1);
        assert_eq!(new, Some(0));
        assert_eq!(priority.total_weight(), 9);
    }
}
