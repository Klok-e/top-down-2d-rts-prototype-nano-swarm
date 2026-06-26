//! Production Facilities and Production Ratio control.
//!
//! Issue #11 contract: production facilities consume delivered
//! resources and produce the type furthest below target ratio.
//! Additional facilities emerge from demand pressure when existing
//! capacity is too busy. Blocked types are skipped temporarily
//! instead of stalling all production.
//!
//! ## State machine
//!
//! Each [`ProductionFacility`] cycles through:
//!
//! ```text
//!   Idle (no current_target)
//!      -> pick deficit type
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
use crate::nanobot::placement::{find_build_zone_placement, BUILDING_FOOTPRINT_RADIUS};
use crate::nanobot::planned::{
    planned_visual_components, PlannedKind, PlannedProductionTarget, PlannedStructure,
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
/// Picked to be a small, deterministic number so tests can drive
/// the simulation with a handful of `app.update()` calls.
pub const PRODUCTION_TICKS_PER_BOT: u32 = 5;

/// Capacity of a [`ProductionFacility`]'s own input hopper. Haulers
/// (logistics leg 3) deliver minerals into this buffer; production
/// consumes exclusively from it. Sized to hold several production
/// cycles' worth so a hauler trip does not stall the facility after
/// one bot, while staying small enough that a cut-off facility
/// drains and goes idle (the maintenance / collapse pressure).
pub const PRODUCTION_INPUT_CAPACITY: u32 = 200;

/// Sum of positive deficits that triggers a new facility to
/// emerge. A small value means a single missing nanobot is
/// already enough to ask for more production capacity, so
/// tests can drive the emergence path with a tight budget.
pub const FACILITY_EMERGE_DEFICIT_THRESHOLD: i32 = 5;

/// Minimum progress an existing facility must have reached in
/// its current cycle before a new facility is allowed to
/// emerge alongside it. This is the "lag" that prevents the
/// auto-creation system from spawning one new facility per
/// tick: a facility that was just picked has progress 0, so
/// the auto-creator skips emergence for at least one tick
/// after the existing facility started producing.
pub const FACILITY_MIN_PROGRESS_FOR_EMERGENCE: u32 = 2;

/// Player-set target production mix. Inserted as a Bevy
/// [`Resource`] so the production systems can read and write it
/// without a public crate API surface.
///
/// The values stored here are **weights**, not target counts
/// (issue #32). The picker normalizes them into shares of the
/// total mix and picks the type whose current population share
/// is furthest below its target share, so the swarm converges
/// on the player's mix regardless of population size.
///
/// The map stores one weight per [`NanobotType`]; `0` means
/// "exclude this type". The total across all types cannot be
/// zero; the slider layer enforces that.
#[derive(Debug, Clone, Resource)]
pub struct ProductionRatio {
    pub weights: HashMap<NanobotType, u32>,
}

impl ProductionRatio {
    /// Empty ratio. Tests use this to set only the types they
    /// care about; the game starts with [`Default`].
    pub fn new() -> Self {
        Self {
            weights: HashMap::new(),
        }
    }

    /// Set the weight for `kind`. No clamping; tests use small
    /// values to keep the deficit math obvious.
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

    /// Fraction of the total mix allocated to `kind`, in
    /// `[0.0, 1.0]`. Returns `0.0` when the total weight is
    /// zero so callers can treat "unset" and "explicitly zero"
    /// identically.
    pub fn normalized_share(&self, kind: NanobotType) -> f32 {
        let total = self.total_weight();
        if total == 0 {
            return 0.0;
        }
        self.weight(kind) as f32 / total as f32
    }

    /// Integer percent (0-100) of the total mix allocated
    /// to `kind`, rounded to the nearest whole number. The
    /// UI uses this for the percentage labels; the production
    /// math itself uses [`Self::normalized_share`].
    pub fn percentage(&self, kind: NanobotType) -> u32 {
        (self.normalized_share(kind) * 100.0).round() as u32
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

impl Default for ProductionRatio {
    fn default() -> Self {
        // Default 60/30/10 mix. Stored as 6/3/1 so the
        // slider's step-5 tick lines up cleanly without
        // forcing the player through 12 clicks to drop a
        // type out of the mix.
        let mut r = Self::new();
        r.set_weight(NanobotType::Worker, 6);
        r.set_weight(NanobotType::Hauler, 3);
        r.set_weight(NanobotType::Defender, 1);
        r
    }
}

/// Marker for a [`crate::nanobot::Swarm`] that is driven by
/// prepainted intent and a fixed production ratio (the project
/// glossary's "Opponent Swarm"). Opponent nanobots still run
/// through the same scoring, logistics, and production systems
/// as the player swarm; the marker only lets callers query
/// opponents separately.
#[derive(Debug, Component, Default)]
pub struct OpponentSwarm {}

/// Per-swarm production ratio override. Attached to a
/// [`crate::nanobot::Swarm`] to give it its own production mix.
/// When present, the production systems prefer this over the
/// global [`ProductionRatio`] resource, so the opponent can
/// keep a fixed mix while the player keeps mutating the global
/// resource.
#[derive(Debug, Component, Clone)]
pub struct SwarmProduction {
    pub ratio: ProductionRatio,
}

impl SwarmProduction {
    pub fn new(ratio: ProductionRatio) -> Self {
        Self { ratio }
    }
}

/// Ties a production facility to the swarm that owns it. Used
/// to resolve the per-swarm [`SwarmProduction`] (or the
/// global [`ProductionRatio`] resource as a fallback) and to
/// decide which swarm a completed cycle spawns its new
/// nanobot under. Facilities without this marker fall back to
/// the global ratio and the first swarm in the world, which
/// keeps the pre-multi-swarm tests working.
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

/// Pick the type with the largest **proportional** deficit
/// (target share - current share), skipping types in `blocked`.
/// Returns `None` when every type is blocked or the total
/// target is zero. Ties are broken by [`NanobotType::ALL`]
/// order, so the picker is deterministic.
///
/// With no current population, every current share is zero
/// and the picker returns the type with the largest target
/// share (the "start with the most-demanded type" rule).
pub fn pick_deficit_type(
    targets: &ProductionRatio,
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

/// Sum of positive share deficits across all types, in
/// **percentage points** (0-300). Each type contributes
/// `max(0, target_share - current_share) * 100`, rounded to
/// the nearest integer. Used by the auto-creation system to
/// detect "production pressure" -- high means the swarm is
/// far from the target mix and existing capacity cannot keep
/// up. Returns `0` when the target has no demand.
pub fn total_deficit(targets: &ProductionRatio, current_counts: &HashMap<NanobotType, u32>) -> i32 {
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
/// swarm's deficit is not muddied by the player swarm's
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

/// Plan a new production facility from demand pressure. For
/// each swarm in the world, a [`PlannedStructure`] of
/// [`PlannedKind::ProductionFacility`] emerges when:
///
/// 1. the swarm's per-swarm deficit (sum of
///    `target - current` across all types, using the swarm's
///    own children as the current count) is at or above
///    [`FACILITY_EMERGE_DEFICIT_THRESHOLD`], AND
/// 2. every existing facility that belongs to this swarm
///    (those with [`OwnerSwarm`] pointing at it, plus any
///    unowned facilities -- the fallback for the
///    pre-multi-swarm case) has a `current_target` and is
///    past [`FACILITY_MIN_PROGRESS_FOR_EMERGENCE`], AND
/// 3. the swarm owns at least one `Build`-painted cell that
///    does not already host a planned or completed
///    structure. The Build Zone is the placement constraint
///    (issue #27 acceptance: "Planned Production Facility
///    placement is constrained to an owned Build Zone").
///
/// The plan carries a [`PlannedProductionTarget`] sidecar
/// recording the type the completed facility should produce
/// first, so the demand layer can pre-allocate the kind
/// that was most under target at planning time. The plan
/// itself does NOT consume any material: build work is
/// worker-time-only in v1, so a Worker can build the plan
/// even when the swarm is short on minerals. The completed
/// `ProductionFacility` then runs through the existing pick
/// + work systems, which consume material on the first
///   pick cycle (or skip the cycle if material is unavailable,
///   matching the existing blocked-type behaviour).
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
    global_ratio: Res<ProductionRatio>,
    nanobots: Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
    facilities: Query<(Entity, &ProductionFacility, Option<&OwnerSwarm>)>,
    existing_targets: Query<
        &Transform,
        Or<(
            With<PlannedStructure>,
            With<ProductionFacility>,
            With<Stockpile>,
            With<crate::nanobot::Charger>,
        )>,
    >,
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
        obstacles.push((pos, BUILDING_FOOTPRINT_RADIUS));
    }
    // Pre-compute the list of (cell, swarm) pairs that are
    // Build-painted, owned by a swarm, and not already
    // occupied. The swarm-by-id map and the per-cell
    // ownership lookup drive the per-swarm placement
    // decision below.
    let mut build_cells_by_swarm: HashMap<SwarmId, Vec<IVec2>> = HashMap::new();
    for (cell, intent_cell) in grid.iter_cells() {
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
        let ratio = swarm_productions
            .get(swarm_entity)
            .map(|sp| &sp.ratio)
            .unwrap_or(&*global_ratio);
        let counts = count_swarm_nanobots_by_type(*swarm_id, &nanobots);
        if total_deficit(ratio, &counts) < FACILITY_EMERGE_DEFICIT_THRESHOLD {
            continue;
        }
        // Facilities that "count" for this swarm: the ones
        // explicitly owned by it plus any unowned
        // facilities. Unowned facilities are kept in the
        // busy-progressed gate so pre-multi-swarm tests --
        // which spawn a facility without an OwnerSwarm --
        // still observe the emergence trigger.
        let relevant: Vec<&ProductionFacility> = facilities
            .iter()
            .filter(|(_, _, owner)| match owner {
                Some(OwnerSwarm(e)) => *e == swarm_entity,
                None => true,
            })
            .map(|(_, f, _)| f)
            .collect();
        if !relevant.is_empty() {
            let all_busy = relevant.iter().all(|f| f.is_busy());
            let all_progressed = relevant
                .iter()
                .all(|f| f.progress >= FACILITY_MIN_PROGRESS_FOR_EMERGENCE);
            if !all_busy || !all_progressed {
                continue;
            }
        }
        let Some(target) = pick_deficit_type(ratio, &counts, &HashSet::new()) else {
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
/// Each facility reads its ratio from the [`SwarmProduction`]
/// of its [`OwnerSwarm`] (if present) or falls back to the
/// global [`ProductionRatio`] resource. Counts are scoped to
/// the owner's children so opponent and player populations
/// cannot leak into each other's deficit math.
///
/// Issue #38 / ADR-0004: counts now match the per-swarm
/// `SwarmId` rather than walking the swarm's `Children`,
/// because nanobots are top-level entities.
#[allow(clippy::type_complexity)]
pub fn production_facility_pick_target_system(
    global_ratio: Res<ProductionRatio>,
    nanobots: Query<(&NanobotType, &crate::nanobot::components::SwarmMember), With<Nanobot>>,
    swarm_productions: Query<&SwarmProduction>,
    swarms: Query<&SwarmId, With<Swarm>>,
    mut facilities: Query<(&mut ProductionFacility, Option<&OwnerSwarm>)>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (mut facility, owner) in &mut facilities {
        if facility.is_busy() {
            continue;
        }

        // Owned facility: owner's ratio and owner's children
        // counts. Unowned facility: global ratio and global
        // population. The unowned branch is the fallback that
        // keeps the pre-multi-swarm tests green.
        //
        // Issue #38 / ADR-0004: the per-swarm count uses
        // the owner's `SwarmId` rather than walking
        // children, because nanobots are top-level
        // entities.
        let (ratio, counts): (&ProductionRatio, HashMap<NanobotType, u32>) = match owner {
            Some(OwnerSwarm(swarm)) => {
                let ratio = swarm_productions
                    .get(*swarm)
                    .map(|sp| &sp.ratio)
                    .unwrap_or(&*global_ratio);
                let swarm_id = swarms.get(*swarm).copied().unwrap_or(SwarmId::PLAYER);
                let counts = count_swarm_nanobots_by_type(swarm_id, &nanobots);
                (ratio, counts)
            }
            None => (&*global_ratio, count_nanobots_by_type(&nanobots)),
        };

        // Try the deficit priority; if the input hopper does
        // not hold a full production cost, block the type and
        // try the next. We loop because blocking one type
        // changes the ranking for the next attempt. The hopper
        // is the facility's own buffer -- a sink stockpile no
        // longer feeds production directly, so leg 3 of the
        // logistics chain (hauler: sink stockpile -> facility)
        // is the only way material reaches this point.
        while let Some(kind) = pick_deficit_type(ratio, &counts, &facility.blocked_types) {
            if facility.input_amount >= PRODUCTION_COST_PER_BOT {
                facility.input_amount -= PRODUCTION_COST_PER_BOT;
                ledger.remove(facility.input_kind, PRODUCTION_COST_PER_BOT);
                facility.current_target = Some(kind);
                facility.progress = 0;
                // The blocked set is preserved for the
                // current cycle: the picker is working
                // through the deficit order, blocking types
                // as it goes. The set clears at the end of
                // the cycle so the next cycle re-tries
                // everything from scratch.
                break;
            }
            facility.blocked_types.insert(kind);
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
    mut facilities: Query<(&mut ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    swarms: Query<(Entity, Option<&SwarmId>), With<Swarm>>,
    opponent_swarms: Query<(), With<OpponentSwarm>>,
    sprites: Option<Res<NanobotSprites>>,
) {
    for (mut facility, transform, owner) in &mut facilities {
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
        app.add_systems(
            Update,
            (
                production_facility_pick_target_system,
                production_facility_work_system,
                production_facility_auto_creation_system
                    .before(crate::nanobot::planned::sink_stockpile_demand_system),
            )
                .chain()
                .after(crate::nanobot::move_velocity_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. End-to-end behaviour lives in
    //! `tests/behavior/production_facility*` and
    //! `tests/behavior/production_ratio_panel.rs`.

    use super::*;

    #[test]
    fn production_ratio_set_and_get_round_trip() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 2);
        r.set_weight(NanobotType::Defender, 1);
        assert_eq!(r.weight(NanobotType::Worker), 5);
        assert_eq!(r.weight(NanobotType::Hauler), 2);
        assert_eq!(r.weight(NanobotType::Defender), 1);
        assert_eq!(r.total_weight(), 8);
    }

    #[test]
    fn production_ratio_unset_returns_zero() {
        // Unset weights must not contribute to the total so
        // the deficit math does not count them as demand.
        let r = ProductionRatio::new();
        assert_eq!(r.weight(NanobotType::Worker), 0);
        assert_eq!(r.total_weight(), 0);
    }

    #[test]
    fn production_ratio_default_seeds_all_three_types() {
        // The default mix exists so a freshly-spawned game
        // has something to converge on; every type must be
        // set or the picker would never produce it.
        let r = ProductionRatio::default();
        assert!(r.weight(NanobotType::Worker) > 0);
        assert!(r.weight(NanobotType::Hauler) > 0);
        assert!(r.weight(NanobotType::Defender) > 0);
    }

    #[test]
    fn pick_deficit_picks_type_with_largest_share_deficit() {
        // 5/0/1 with target 5/10/5: current shares 5/10
        // Worker=over, 0/10 Hauler=most-under, 1/5
        // Defender=under. Picker must choose Hauler.
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 10);
        r.set_weight(NanobotType::Defender, 5);
        let mut counts = HashMap::new();
        counts.insert(NanobotType::Worker, 5);
        counts.insert(NanobotType::Hauler, 0);
        counts.insert(NanobotType::Defender, 1);
        assert_eq!(
            pick_deficit_type(&r, &counts, &HashSet::new()),
            Some(NanobotType::Hauler)
        );
    }

    #[test]
    fn pick_deficit_tie_breaks_in_stable_order() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 5);
        r.set_weight(NanobotType::Defender, 5);
        let counts = HashMap::new();
        // All zero populations, all equal target share --
        // first type in `NanobotType::ALL` wins.
        assert_eq!(
            pick_deficit_type(&r, &counts, &HashSet::new()),
            Some(NanobotType::Worker)
        );
    }

    #[test]
    fn pick_deficit_returns_none_when_all_at_or_above_share() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        let mut counts = HashMap::new();
        counts.insert(NanobotType::Worker, 5);
        let blocked = HashSet::new();
        assert_eq!(pick_deficit_type(&r, &counts, &blocked), None);
        // Surplus also counts as nothing to do.
        counts.insert(NanobotType::Worker, 8);
        assert_eq!(pick_deficit_type(&r, &counts, &blocked), None);
    }

    #[test]
    fn pick_deficit_skips_blocked_type() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 5);
        let counts = HashMap::new();
        let mut blocked = HashSet::new();
        blocked.insert(NanobotType::Worker);
        assert_eq!(
            pick_deficit_type(&r, &counts, &blocked),
            Some(NanobotType::Hauler)
        );
    }

    #[test]
    fn pick_deficit_returns_none_when_all_blocked() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 5);
        let counts = HashMap::new();
        let blocked: HashSet<_> = NanobotType::ALL.iter().copied().collect();
        assert_eq!(pick_deficit_type(&r, &counts, &blocked), None);
    }

    #[test]
    fn total_deficit_sums_positive_share_gaps_in_percentage_points() {
        // Weights 5/3/0 -> target shares 62.5/37.5/0%.
        // Counts 2/5/0 (total 7) -> current shares
        // 28.57/71.43/0%. Only Worker is under; deficit is
        // 62.5 - 28.57 = 33.93 pp, rounds to 34.
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 3);
        r.set_weight(NanobotType::Defender, 0);
        let mut counts = HashMap::new();
        counts.insert(NanobotType::Worker, 2);
        counts.insert(NanobotType::Hauler, 5);
        counts.insert(NanobotType::Defender, 0);
        assert_eq!(total_deficit(&r, &counts), 34);
    }

    #[test]
    fn total_deficit_zero_when_all_met() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 5);
        r.set_weight(NanobotType::Hauler, 3);
        let mut counts = HashMap::new();
        counts.insert(NanobotType::Worker, 5);
        counts.insert(NanobotType::Hauler, 3);
        assert_eq!(total_deficit(&r, &counts), 0);
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
    fn swarm_production_wraps_a_production_ratio() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Hauler, 4);
        let sp = SwarmProduction::new(r.clone());
        assert_eq!(sp.ratio.weight(NanobotType::Hauler), 4);
    }

    #[test]
    fn owner_swarm_stores_the_entity_reference() {
        let mut world = World::new();
        let swarm = world.spawn_empty().id();
        let owner = OwnerSwarm(swarm);
        assert_eq!(owner.0, swarm);
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
        f.progress = 2;
        assert_eq!(production_progress_percent(&f), 40);
        f.progress = PRODUCTION_TICKS_PER_BOT;
        assert_eq!(production_progress_percent(&f), 100);
        // Defensive: progress over the budget must not
        // report >100%.
        f.progress = PRODUCTION_TICKS_PER_BOT + 5;
        assert_eq!(production_progress_percent(&f), 100);
    }

    // ---- Issue #32: weights, proportional picker, clamp ----

    #[test]
    fn production_ratio_default_seeds_60_30_10_normalized() {
        let r = ProductionRatio::default();
        assert_eq!(r.normalized_share(NanobotType::Worker), 0.60);
        assert_eq!(r.normalized_share(NanobotType::Hauler), 0.30);
        assert_eq!(r.normalized_share(NanobotType::Defender), 0.10);
    }

    #[test]
    fn production_ratio_normalized_share_is_zero_when_total_is_zero() {
        // Avoids NaN from divide-by-zero.
        let r = ProductionRatio::new();
        assert_eq!(r.normalized_share(NanobotType::Worker), 0.0);
        assert_eq!(r.normalized_share(NanobotType::Hauler), 0.0);
        assert_eq!(r.normalized_share(NanobotType::Defender), 0.0);
    }

    #[test]
    fn production_ratio_normalized_share_matches_weight_fraction() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 7);
        r.set_weight(NanobotType::Hauler, 3);
        assert!((r.normalized_share(NanobotType::Worker) - 0.7).abs() < 1e-6);
        assert!((r.normalized_share(NanobotType::Hauler) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn production_ratio_try_change_weight_applies_positive_delta() {
        let mut r = ProductionRatio::default();
        let new = r.try_change_weight(NanobotType::Worker, 5);
        assert_eq!(new, Some(11));
        assert_eq!(r.weight(NanobotType::Worker), 11);
    }

    #[test]
    fn production_ratio_try_change_weight_applies_negative_delta() {
        let mut r = ProductionRatio::default();
        let new = r.try_change_weight(NanobotType::Hauler, -3);
        assert_eq!(new, Some(0));
        assert_eq!(r.weight(NanobotType::Hauler), 0);
    }

    #[test]
    fn production_ratio_try_change_weight_clamps_to_zero() {
        // Negative delta larger than current must saturate
        // at 0, never underflow. The "total cannot become
        // zero" rule is checked in the next test.
        let mut r = ProductionRatio::default();
        let new = r.try_change_weight(NanobotType::Defender, -100);
        assert_eq!(new, Some(0));
        assert_eq!(r.weight(NanobotType::Defender), 0);
    }

    #[test]
    fn production_ratio_try_change_weight_rejects_zero_total() {
        // Acceptance: "the last nonzero type is clamped to
        // a nonzero value." With only Defender set, dropping
        // it to 0 would zero the total -- rejected, value
        // stays at 1.
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Defender, 1);
        let new = r.try_change_weight(NanobotType::Defender, -1);
        assert_eq!(new, None);
        assert_eq!(r.weight(NanobotType::Defender), 1);
    }

    #[test]
    fn production_ratio_try_change_weight_allows_zero_when_other_types_remain() {
        let mut r = ProductionRatio::default();
        let new = r.try_change_weight(NanobotType::Defender, -1);
        assert_eq!(new, Some(0));
        assert_eq!(r.total_weight(), 9);
    }

    #[test]
    fn pick_deficit_issue_example_w8_h1_d1_target_60_30_10_picks_hauler() {
        // Acceptance: W8 H1 D1 with target 60/30/10 ->
        // Hauler (current 10% share vs 30% target, biggest
        // under-share).
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 60);
        r.set_weight(NanobotType::Hauler, 30);
        r.set_weight(NanobotType::Defender, 10);
        let mut counts = HashMap::new();
        counts.insert(NanobotType::Worker, 8);
        counts.insert(NanobotType::Hauler, 1);
        counts.insert(NanobotType::Defender, 1);
        assert_eq!(
            pick_deficit_type(&r, &counts, &HashSet::new()),
            Some(NanobotType::Hauler)
        );
    }

    #[test]
    fn pick_deficit_proportional_ignores_population_scale() {
        // Same current shares at two scales must pick the
        // same next type.
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 6);
        r.set_weight(NanobotType::Hauler, 3);
        r.set_weight(NanobotType::Defender, 1);
        let small: HashMap<_, _> = [
            (NanobotType::Worker, 4),
            (NanobotType::Hauler, 1),
            (NanobotType::Defender, 0),
        ]
        .into_iter()
        .collect();
        let large: HashMap<_, _> = [
            (NanobotType::Worker, 40),
            (NanobotType::Hauler, 10),
            (NanobotType::Defender, 0),
        ]
        .into_iter()
        .collect();
        let small_pick = pick_deficit_type(&r, &small, &HashSet::new());
        let large_pick = pick_deficit_type(&r, &large, &HashSet::new());
        assert_eq!(small_pick, Some(NanobotType::Hauler));
        assert_eq!(large_pick, small_pick);
    }
}
