//! Gather behavior for Worker nanobots.
//!
//! Issue #7 contract: workers score Gather intent through the
//! dumb-autonomy path (issue #6), extract from deposits in painted
//! Gather cells, carry a small physical load, and drop it at a
//! stockpile. The Gather Zone itself is never cleared on depletion;
//! a refilled deposit pulls idle workers back through the same
//! scoring path.
//!
//! State machine carried on the worker by marker components:
//!
//! ```text
//!   Idle -> (assignment system) -> Moving (GatherAssignment + DMC)
//!   Moving -> (arrive system) -> Extracting (GatherAssignment + ExtractProgress)
//!   Extracting -> (extract system) -> Carrying (WorkerLoad)
//!   Carrying -> (carry_assign system) -> Delivering (WorkerLoad + ReturningToStockpile + DMC)
//!   Delivering -> (delivery system) -> Idle
//! ```
//!
//! Soft work slot occupancy is bumped on assignment and released
//! when the worker leaves the gather cell, so the issue #6 slot
//! pressure stays in sync with the number of workers actually
//! working a given cell.

use bevy::prelude::*;

use crate::ai::get_world_from_zone;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{best_candidate, Commitment, NanobotType, SoftWorkSlots};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, Swarm, SwarmId, SwarmMember};
use crate::nanobot::placement::{
    find_source_stockpile_placement, SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
    SOURCE_STOCKPILE_JITTER_AMPLITUDE, SOURCE_STOCKPILE_PADDING, SOURCE_STOCKPILE_PLACEMENT_COUNT,
    SOURCE_STOCKPILE_PLACEMENT_RADIUS,
};
use crate::nanobot::planned::{
    planned_visual_components, PlannedKind, PlannedStructure, PlannedStructureClaim,
    PlannedStructureProgress,
};
use crate::nanobot::production::OwnerSwarm;
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole};
use crate::structure_sprites::StructureSprites;
use crate::ZONE_BLOCK_SIZE;

/// Maximum units a Worker can carry in a single trip. The glossary
/// is explicit: Workers carry "small" amounts; Haulers carry more.
/// Four units is a deliberately small number so the trip is
/// visible (the worker leaves with a partial load and comes back
/// for more) and the test math is obvious.
pub const WORKER_CARRY_CAPACITY: u32 = 4;

/// Units extracted per `app.update()` tick. Fixed instead of
/// time-based so tests can drive the simulation with deterministic
/// `app.update()` calls. The real game can scale this with
/// `Time::delta_secs()` once the simulation has a real clock.
pub const EXTRACT_PER_TICK: u32 = 1;

/// Maximum distance (world units) from a Resource Deposit at
/// which a Source Stockpile is considered "near" the deposit.
///
/// Picked at three quarters of `ZONE_BLOCK_SIZE` (384 < 512)
/// so a Source Stockpile placed at the canonical offset from
/// a deposit inside one cell stays inside that same cell. The
/// "near" check is the same for both the demand system (which
/// decides whether to plan another Source Stockpile) and the
/// gather arrive system (which decides whether the worker has
/// a usable built Source Stockpile to deliver into).
pub const SOURCE_STOCKPILE_PROXIMITY_RADIUS: f32 = 384.0;

/// World-space offset from a Resource Deposit's center where
/// the Source Stockpile demand system places a new
/// `PlannedStructure`. Picked so the planned structure's
/// footprint (`PLANNED_STRUCTURE_FOOTPRINT` = 64) does not
/// overlap the deposit's circle (default radius 32): the
/// planned structure's centre is 96 units from the deposit's
/// centre, leaving a 32-unit gap between the deposit's edge
/// and the planned structure's edge.
pub const SOURCE_STOCKPILE_OFFSET: Vec2 = Vec2::new(96.0, 0.0);

/// What a Worker is currently carrying. The component is only
/// present with `amount > 0`: it is inserted on extraction
/// completion and removed on delivery, so absence means the worker
/// is idle or doing other work.
#[derive(Debug, Component, Clone, Copy)]
pub struct WorkerLoad {
    pub kind: ResourceKind,
    pub amount: u32,
}

/// Marks a Worker as committed to a specific deposit in a specific
/// Gather cell. Set by the assignment system, cleared when the
/// worker transitions to Carrying or when the deposit disappears.
#[derive(Debug, Component, Clone, Copy)]
pub struct GatherAssignment {
    pub cell: IVec2,
    pub deposit: Entity,
}

impl GatherAssignment {
    pub fn new(cell: IVec2, deposit: Entity) -> Self {
        Self { cell, deposit }
    }
}

/// In-flight extraction progress. Lives only while the worker is
/// standing at the assigned deposit and pulling resources. The
/// `collected` count caps at [`WORKER_CARRY_CAPACITY`]; reaching
/// the cap transitions the worker to Carrying.
#[derive(Debug, Component, Default, Clone, Copy)]
pub struct ExtractProgress {
    pub collected: u32,
}

/// Marks a Worker that is carrying a load toward a specific
/// stockpile. Set by the carry-assign system, cleared when the
/// worker reaches the stockpile and drops the load.
#[derive(Debug, Component, Clone, Copy)]
pub struct ReturningToStockpile {
    pub stockpile: Entity,
}

pub fn world_to_cell(world: Vec2) -> IVec2 {
    IVec2::new(
        (world.x / ZONE_BLOCK_SIZE).floor() as i32,
        (world.y / ZONE_BLOCK_SIZE).floor() as i32,
    )
}

/// True when the circle (`circle_center`, `circle_radius`) visually
/// overlaps the rectangle of `cell` in the intent grid. A cell at
/// `(i, j)` spans world coordinates
/// `[i * ZONE_BLOCK_SIZE, (i + 1) * ZONE_BLOCK_SIZE)` on x and
/// `[j * ZONE_BLOCK_SIZE, (j + 1) * ZONE_BLOCK_SIZE)` on y. The
/// standard "closest point on the rect to the circle center" test
/// gives strict overlap when the distance from the rect to the
/// center is `< radius`, and a touch (radius == distance) also
/// counts as overlap so a deposit circle that just reaches the
/// cell border still makes the deposit eligible.
///
/// Issue #22 contract: a Resource Deposit is eligible for gather
/// work when its circular work area intersects a painted Gather
/// cell owned by the same swarm. Paint strength still affects
/// scoring, but the eligibility gate is geometric overlap, not
/// exact intent-grid cell membership.
pub fn cell_overlaps_circle(cell: IVec2, circle_center: Vec2, circle_radius: f32) -> bool {
    let min = Vec2::new(
        cell.x as f32 * ZONE_BLOCK_SIZE,
        cell.y as f32 * ZONE_BLOCK_SIZE,
    );
    let max = Vec2::new(
        (cell.x + 1) as f32 * ZONE_BLOCK_SIZE,
        (cell.y + 1) as f32 * ZONE_BLOCK_SIZE,
    );
    let closest_x = circle_center.x.clamp(min.x, max.x);
    let closest_y = circle_center.y.clamp(min.y, max.y);
    let dx = circle_center.x - closest_x;
    let dy = circle_center.y - closest_y;
    dx * dx + dy * dy <= circle_radius * circle_radius
}

fn find_nearest_deposit_in_cell(
    cell: IVec2,
    kind: ResourceKind,
    worker_pos: Vec2,
    deposits: &Query<(Entity, &ResourceDeposit, &Transform)>,
) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for (entity, deposit, transform) in deposits.iter() {
        if deposit.kind != kind || deposit.amount == 0 {
            continue;
        }
        // Issue #22: visual overlap with the painted cell's
        // rectangle, not exact intent-grid cell membership.
        let deposit_pos = transform.translation.truncate();
        if !cell_overlaps_circle(cell, deposit_pos, deposit.radius) {
            continue;
        }
        let d = worker_pos.distance(deposit_pos);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, entity));
        }
    }
    best.map(|(_, e)| e)
}

fn find_nearest_stockpile(
    kind: ResourceKind,
    worker_pos: Vec2,
    stockpiles: &Query<(Entity, &Stockpile, &Transform)>,
) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for (entity, stockpile, transform) in stockpiles.iter() {
        if stockpile.kind != kind {
            continue;
        }
        if stockpile.free_space() == 0 {
            continue;
        }
        let d = worker_pos.distance(transform.translation.truncate());
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, entity));
        }
    }
    best.map(|(_, e)| e)
}

/// True when a built [`Stockpile`] of `kind` with free space
/// is within [`SOURCE_STOCKPILE_PROXIMITY_RADIUS`] of
/// `deposit_pos`. This is the "worker can extract and deliver
/// to a usable Source Stockpile right now" half of the gather
/// contract; a planned (not yet built) Source Stockpile does
/// not count as usable here because the worker has nowhere to
/// drop the carried load until the planned structure promotes
/// to a completed `Stockpile`.
///
/// ## Issue #26: Source vs Sink filter
///
/// A [`Stockpile`] carrying [`StockpileRole::Sink`] lives in
/// a Build cell (base infrastructure) and is not a valid
/// destination for a gather worker's tiny load. A bare
/// `Stockpile` without an explicit role marker still counts
/// (the legacy default is `Source`); only the explicitly
/// `Sink`-stamped ones are excluded, so pre-existing tests
/// that spawn `Stockpile` entities directly keep passing.
pub(crate) fn has_usable_built_source_stockpile(
    deposit_pos: Vec2,
    stockpiles: &Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
) -> bool {
    stockpiles.iter().any(|(s, t, role)| {
        s.kind == ResourceKind::Minerals
            && s.free_space() > 0
            && !matches!(role, Some(StockpileRole::Sink))
            && t.translation.truncate().distance(deposit_pos) <= SOURCE_STOCKPILE_PROXIMITY_RADIUS
    })
}

/// True when a built Source Stockpile (per
/// [`has_usable_built_source_stockpile`]) OR a planned Source
/// Stockpile is within [`SOURCE_STOCKPILE_PROXIMITY_RADIUS`]
/// of `deposit_pos`. This is the "demand is already
/// satisfied, do not plan another" half of the contract. The
/// gather arrive system uses the stricter built-only check so
/// the worker only extracts when delivery is actually
/// possible; the demand system uses this looser check so a
/// pending planned structure counts as demand satisfied and
/// the swarm does not pile multiple Source Stockpile plans
/// around the same deposit.
///
/// ## Issue #25: full-stockpile expansion
///
/// Both checks use the same "usable" definition: a built
/// stockpile with `free_space() > 0` is "usable", a full one
/// is not. So when the only nearby built Source Stockpiles
/// are full, the demand system sees no usable destination and
/// plans another Source Stockpile rather than treating the
/// saturated site as "demand satisfied". The carry-assign and
/// delivery systems apply the same `free_space() > 0` filter,
/// so a Worker carrying a load also ignores full stockpiles
/// and waits for a usable destination.
///
/// ## Issue #26: Source vs Sink filter
///
/// A `Sink` Stockpile in the same area does NOT count as a
/// usable Source. The role filter is applied in
/// [`has_usable_built_source_stockpile`] for the built check;
/// a planned structure's `kind` is the equivalent filter for
/// the pending check, since the planned-structure
/// auto-creation system plans `SinkStockpile` for Build
/// cells (not for the gather site) and the demand system
/// plans `SourceStockpile` for Gather cells. A Sink
/// Stockpile's planned form would never enter this query
/// because its world position is in a Build cell, not next
/// to a `ResourceDeposit`.
///
/// `newly_planned` is the set of positions where this same
/// demand system has just spawned a planned structure on
/// this tick. Bevy [`Commands`] are deferred, so the live
/// `planned` query cannot see them yet; passing the in-tick
/// positions keeps the "near" check correct within a single
/// tick.
pub(crate) fn has_any_near_source_stockpile(
    deposit_pos: Vec2,
    stockpiles: &Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
    planned: &Query<(&PlannedStructure, &Transform)>,
    newly_planned: &[Vec2],
) -> bool {
    if has_usable_built_source_stockpile(deposit_pos, stockpiles) {
        return true;
    }
    if planned.iter().any(|(p, t)| {
        p.kind == PlannedKind::SourceStockpile
            && t.translation.truncate().distance(deposit_pos) <= SOURCE_STOCKPILE_PROXIMITY_RADIUS
    }) {
        return true;
    }
    newly_planned
        .iter()
        .any(|pos| pos.distance(deposit_pos) <= SOURCE_STOCKPILE_PROXIMITY_RADIUS)
}

/// For each unique deposit that has at least one Worker with a
/// [`GatherAssignment`], ensure a Planned Source Stockpile
/// exists within [`SOURCE_STOCKPILE_PROXIMITY_RADIUS`] of the
/// deposit.
///
/// The demand system is the "Gather intent asks for a Source
/// Stockpile" half of the issue #23 contract. A Worker that
/// arrives at a deposit with no usable built Source Stockpile
/// pauses extraction; the planned structure's claim system
/// then picks up the planned structure (the same gather Worker
/// is the natural claimer when it is the only idle worker)
/// and the build proceeds. Once the planned structure
/// promotes to a completed [`Stockpile`], the gather arrive
/// system sees the usable Source Stockpile and the Worker
/// resumes extraction.
///
/// The demand system is "demand-driven" rather than
/// "intent-driven": a Planned Source Stockpile is only created
/// when a Worker is actually assigned to the deposit, not
/// when Gather paint is applied. This matches the acceptance
/// criterion "no completed Source Stockpile appears
/// instantly from Gather paint alone" -- the visible result
/// of painting Gather intent is a planned structure, not a
/// completed stockpile, and it only appears once a Worker has
/// been routed to the deposit.
///
/// The "reused" half of the contract: if a planned Source
/// Stockpile already exists within proximity, the demand
/// system does not plan another. Two Workers assigned to the
/// same deposit (or to two deposits in the same area) share
/// the single planned structure. The reuse check accounts
/// for two sources of "existing" planned structures: the
/// live ECS query (structures spawned on a previous tick)
/// and a local set of positions planned on this same tick
/// (Bevy [`Commands`] are deferred, so a planned structure
/// spawned earlier in this tick is not yet visible to the
/// query but is still "real" for the reuse check). See
/// [`has_any_near_source_stockpile`] for the full-stockpile
/// expansion half of the contract.
///
/// ## Placement (issue #24)
///
/// The position of a new planned structure is chosen by
/// [`find_source_stockpile_placement`]. See that function
/// and the `nanobot::placement` module docs for the full
/// algorithm; in short, candidates are generated on a ring
/// at [`SOURCE_STOCKPILE_PLACEMENT_RADIUS`] from the
/// deposit, jittered deterministically, filtered by
/// Gather-Zone containment and overlap (with
/// [`SOURCE_STOCKPILE_PADDING`]), and scored by alignment
/// with the expected haul direction. When every candidate
/// is rejected, no planned structure is created and the
/// demand is retried on a later tick.
///
/// Ownership: the planned structure is stamped with
/// [`OwnerSwarm`] using the first [`Swarm`] in the world,
/// matching the existing planned-structure auto-creation
/// pattern from issue #21. The promotion path preserves
/// `OwnerSwarm` on the completed Stockpile, so the
/// "Source Stockpile owned by the same swarm" contract
/// holds end-to-end. A per-swarm filter tied to the painted
/// cell's intent owner is a follow-up; the v1 simulation
/// has one swarm.
#[allow(clippy::too_many_arguments)]
pub fn source_stockpile_demand_system(
    mut commands: Commands,
    structure_sprites: Res<StructureSprites>,
    gather_assignments: Query<(&GatherAssignment, &SwarmMember)>,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    stockpiles: Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
    planned: Query<(&PlannedStructure, &Transform)>,
    swarms: Query<(Entity, &SwarmId, &Transform), With<Swarm>>,
    grid: Res<IntentGrid>,
) {
    let swarm_by_id: std::collections::HashMap<SwarmId, (Entity, Vec2)> = swarms
        .iter()
        .map(|(entity, id, transform)| (*id, (entity, transform.translation.truncate())))
        .collect();
    let mut deposits_seen: std::collections::HashMap<Entity, SwarmId> =
        std::collections::HashMap::new();
    for (assignment, swarm_member) in &gather_assignments {
        deposits_seen
            .entry(assignment.deposit)
            .or_insert(swarm_member.0);
    }
    // Positions of Planned Source Stockpiles planned earlier
    // in this tick. The placement algorithm treats them as
    // obstacles so two deposits processed in the same tick
    // share a single planned structure (Bevy `Commands` are
    // deferred, so the live `planned` query cannot see the
    // in-tick spawns yet).
    let mut newly_planned_positions: Vec<Vec2> = Vec::new();
    for (deposit_entity, demand_swarm) in deposits_seen {
        let Ok((deposit, deposit_transform)) = deposits.get(deposit_entity) else {
            continue;
        };
        let deposit_pos = deposit_transform.translation.truncate();
        if has_any_near_source_stockpile(
            deposit_pos,
            &stockpiles,
            &planned,
            &newly_planned_positions,
        ) {
            continue;
        }
        // Build the obstacle list for this deposit: every
        // existing Source Stockpile and every planned
        // structure, plus any in-tick positions from this
        // loop. Each entry carries its own center and
        // half-footprint so the placement algorithm can
        // generalize to mixed-size obstacles in the future.
        let mut obstacles: Vec<(Vec2, f32)> = Vec::new();
        obstacles.extend(
            deposits
                .iter()
                .map(|(deposit, t)| (t.translation.truncate(), deposit.radius)),
        );
        obstacles.extend(
            stockpiles
                .iter()
                .map(|(_, t, _)| (t.translation.truncate(), SOURCE_STOCKPILE_FOOTPRINT_RADIUS)),
        );
        obstacles.extend(
            planned
                .iter()
                .map(|(_, t)| (t.translation.truncate(), SOURCE_STOCKPILE_FOOTPRINT_RADIUS)),
        );
        obstacles.extend(
            newly_planned_positions
                .iter()
                .map(|p| (*p, SOURCE_STOCKPILE_FOOTPRINT_RADIUS)),
        );
        let mut gather_cells: Vec<IVec2> = Vec::new();
        let mut build_worlds: Vec<Vec2> = Vec::new();
        for (cell, intent_cell) in grid.iter_cells() {
            if intent_cell.has(IntentKind::Gather)
                && intent_cell
                    .owner(IntentKind::Gather)
                    .is_none_or(|owner| owner == demand_swarm)
            {
                gather_cells.push(cell);
            }
            if intent_cell.has(IntentKind::Build)
                && intent_cell
                    .owner(IntentKind::Build)
                    .is_none_or(|owner| owner == demand_swarm)
            {
                build_worlds.push(get_world_from_zone(cell));
            }
        }
        let swarm_origin = swarm_by_id.get(&demand_swarm).map(|(_, pos)| *pos);
        let haul_direction = compute_haul_direction(deposit_pos, &build_worlds, swarm_origin);
        // Find a valid placement for the planned structure.
        // `None` here means every candidate was rejected by
        // the zone / overlap filter; the demand remains
        // unsatisfied and is retried on a later tick.
        let required_clearance =
            deposit.radius + SOURCE_STOCKPILE_FOOTPRINT_RADIUS + SOURCE_STOCKPILE_PADDING;
        let placement_radius = if required_clearance > SOURCE_STOCKPILE_PLACEMENT_RADIUS {
            required_clearance + SOURCE_STOCKPILE_JITTER_AMPLITUDE * 2.0
        } else {
            SOURCE_STOCKPILE_PLACEMENT_RADIUS
        };
        let Some(placement_pos) = find_source_stockpile_placement(
            deposit_pos,
            &gather_cells,
            &obstacles,
            haul_direction,
            placement_radius,
            SOURCE_STOCKPILE_PLACEMENT_COUNT,
            SOURCE_STOCKPILE_JITTER_AMPLITUDE,
            SOURCE_STOCKPILE_FOOTPRINT_RADIUS,
            SOURCE_STOCKPILE_PADDING,
        ) else {
            continue;
        };
        let placement_cell = world_to_cell(placement_pos);
        newly_planned_positions.push(placement_pos);
        let mut entity_commands = commands.spawn((
            PlannedStructure::new(PlannedKind::SourceStockpile, placement_cell),
            planned_visual_components(
                PlannedKind::SourceStockpile,
                &structure_sprites,
                placement_pos,
            ),
        ));
        if let Some((swarm_entity, _)) = swarm_by_id.get(&demand_swarm) {
            entity_commands.insert(OwnerSwarm(*swarm_entity));
        }
    }
}

/// Compute the expected haul direction from `deposit_pos`. The
/// direction is the unit vector from the deposit to the
/// nearest Build-painted cell's world center (in `build_worlds`),
/// with a fallback to `swarm_origin` and a final fallback to a
/// zero vector (no bias). The Build cell wins because the
/// PRD's "expected haul direction should prefer a Build Zone
/// or base/sink direction" rule treats the Build Zone as the
/// primary sink; the swarm origin is the same destination
/// when no Build Zone has been painted yet.
fn compute_haul_direction(
    deposit_pos: Vec2,
    build_worlds: &[Vec2],
    swarm_origin: Option<Vec2>,
) -> Vec2 {
    let nearest = build_worlds
        .iter()
        .min_by(|a, b| {
            deposit_pos
                .distance(**a)
                .partial_cmp(&deposit_pos.distance(**b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied();
    if let Some(target) = nearest {
        let delta = target - deposit_pos;
        if delta.length() > f32::EPSILON {
            return delta.normalize();
        }
    }
    if let Some(origin) = swarm_origin {
        let delta = origin - deposit_pos;
        if delta.length() > f32::EPSILON {
            return delta.normalize();
        }
    }
    // No Build cell, no swarm origin, or both are
    // coincident with the deposit: the haul direction is
    // unknown, so the placement algorithm falls back to its
    // angle-index tie-breaker.
    Vec2::ZERO
}

/// For each idle Worker with no in-flight gather or carry work, pick
/// a Gather cell through the autonomy scoring from issue #6 and
/// assign the closest deposit in that cell. The (cell, Gather) soft
/// work slot is occupied so future assignees see the cell as busier.
#[allow(clippy::type_complexity)]
pub fn worker_gather_assignment_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    mut slots: ResMut<SoftWorkSlots>,
    workers: Query<
        (Entity, &Transform, &Commitment, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            Without<GatherAssignment>,
            Without<ExtractProgress>,
            Without<ReturningToStockpile>,
            Without<DirectMovementComponent>,
            Without<WorkerLoad>,
        ),
    >,
    deposits: Query<(Entity, &ResourceDeposit, &Transform)>,
) {
    // Snapshot the slot counts once per tick so the scoring
    // function reads a consistent view while we mutate the
    // resource below.
    let slots_snapshot = slots.clone();
    for (entity, transform, commitment, nanobot_type, swarm_member) in &workers {
        if *nanobot_type != NanobotType::Worker {
            continue;
        }
        if *commitment != Commitment::Idle {
            continue;
        }

        let worker_pos = transform.translation.truncate();
        let Some(candidate) = best_candidate(
            &grid,
            NanobotType::Worker,
            *commitment,
            worker_pos,
            &slots_snapshot,
            ZONE_BLOCK_SIZE,
            &[IntentKind::Gather],
            swarm_member.0,
        ) else {
            continue;
        };
        if candidate.kind != IntentKind::Gather {
            continue;
        }

        let Some(deposit_entity) = find_nearest_deposit_in_cell(
            candidate.cell,
            ResourceKind::Minerals,
            worker_pos,
            &deposits,
        ) else {
            // No deposit in the painted cell. The Gather Zone
            // still stands (it persists across depletion), so the
            // worker stays idle. A refill on this cell will be
            // picked up on a later tick.
            continue;
        };

        let Ok((_, _, deposit_transform)) = deposits.get(deposit_entity) else {
            continue;
        };

        slots.occupy(candidate.cell, IntentKind::Gather);
        commands.entity(entity).insert((
            GatherAssignment::new(candidate.cell, deposit_entity),
            DirectMovementComponent {
                xy: deposit_transform.translation.truncate(),
            },
        ));
    }
}

/// Detect a worker that has arrived at its assigned deposit and
/// start the extraction phase. The `Without<ExtractProgress>` filter
/// makes arrival idempotent -- the same tick cannot fire twice.
///
/// Issue #23 contract: a Worker does not start extracting until a
/// usable built Source Stockpile exists within
/// [`SOURCE_STOCKPILE_PROXIMITY_RADIUS`] of the deposit. If no
/// built stockpile exists, the worker stays put; the
/// [`source_stockpile_demand_system`] has already (or will on a
/// subsequent tick) planned a Source Stockpile, and the planned
/// structure's claim system will route a worker -- often this
/// same worker once it is the only idle one -- to build it.
///
/// The `Without<PlannedStructureClaim>` and
/// `Without<PlannedStructureProgress>` filters ensure the
/// arrive system does not interfere with a worker that has
/// been claimed to build a nearby planned Source Stockpile;
/// the planned structure's own arrive/work systems handle
/// that worker's state machine.
///
/// When the worker is not at the deposit (for example, after
/// completing a build and walking back) the system re-issues
/// a [`DirectMovementComponent`] pointing at the deposit so
/// the movement system routes it there. This is the "resume
/// extraction after the Source Stockpile exists" half of the
/// contract.
#[allow(clippy::type_complexity)]
pub fn worker_gather_arrive_system(
    mut commands: Commands,
    mut slots: ResMut<SoftWorkSlots>,
    workers: Query<
        (Entity, &Transform, &GatherAssignment),
        (
            With<Nanobot>,
            With<GatherAssignment>,
            Without<DirectMovementComponent>,
            Without<ExtractProgress>,
            Without<PlannedStructureClaim>,
            Without<PlannedStructureProgress>,
        ),
    >,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    stockpiles: Query<(&Stockpile, &Transform, Option<&StockpileRole>)>,
) {
    for (entity, transform, assignment) in &workers {
        let Ok((deposit, deposit_transform)) = deposits.get(assignment.deposit) else {
            // Deposit is gone (e.g. consumed by a future system).
            // Release the slot and drop the assignment; the
            // Gather Zone stays painted.
            slots.release(assignment.cell, IntentKind::Gather);
            commands.entity(entity).remove::<GatherAssignment>();
            continue;
        };
        if deposit.amount == 0 {
            // Deposit drained between assignment and arrival.
            // The Gather Zone stays painted; the worker idles.
            slots.release(assignment.cell, IntentKind::Gather);
            commands.entity(entity).remove::<GatherAssignment>();
            continue;
        }

        let worker_pos = transform.translation.truncate();
        let deposit_pos = deposit_transform.translation.truncate();
        let distance = worker_pos.distance(deposit_pos);
        if distance > deposit.radius {
            // Not at the deposit yet (e.g. after completing a
            // build and walking back). Re-issue the movement
            // command so the worker returns. Commands are
            // idempotent on a stale `DirectMovementComponent`
            // because the system that already pruned the
            // component only runs on arrival, not every tick.
            commands
                .entity(entity)
                .insert(DirectMovementComponent { xy: deposit_pos });
            continue;
        }
        if !has_usable_built_source_stockpile(deposit_pos, &stockpiles) {
            // No usable built Source Stockpile. The demand
            // system has already (or will) plan one. The
            // planned structure's claim system will route a
            // worker to build it; this worker stays put until
            // the planned structure promotes to a real
            // stockpile and the next tick's arrive check
            // passes.
            continue;
        }
        commands.entity(entity).insert(ExtractProgress::default());
    }
}

/// Drain `EXTRACT_PER_TICK` units from the assigned deposit every
/// tick while the worker is at the deposit and the load is not
/// full. When the load is full or the deposit empties (or
/// disappears), transition the worker to Carrying.
#[allow(clippy::type_complexity)]
pub fn worker_gather_extract_system(
    mut commands: Commands,
    mut slots: ResMut<SoftWorkSlots>,
    mut workers: Query<(Entity, &mut ExtractProgress, &GatherAssignment), With<Nanobot>>,
    mut deposits: Query<&mut ResourceDeposit>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, mut progress, assignment) in &mut workers {
        let Ok(mut deposit) = deposits.get_mut(assignment.deposit) else {
            transition_to_carrying(
                &mut commands,
                entity,
                assignment.cell,
                progress.collected,
                &mut slots,
            );
            continue;
        };
        if deposit.amount == 0 {
            // A partial load is still useful; carry it to a
            // stockpile rather than abandoning it.
            transition_to_carrying(
                &mut commands,
                entity,
                assignment.cell,
                progress.collected,
                &mut slots,
            );
            continue;
        }
        if progress.collected >= WORKER_CARRY_CAPACITY {
            // Small load cap; transition even if the deposit
            // still has resources.
            transition_to_carrying(
                &mut commands,
                entity,
                assignment.cell,
                progress.collected,
                &mut slots,
            );
            continue;
        }

        let can_still_carry = WORKER_CARRY_CAPACITY - progress.collected;
        let actual = EXTRACT_PER_TICK.min(deposit.amount).min(can_still_carry);
        progress.collected += actual;
        deposit.amount -= actual;
        ledger.remove(deposit.kind, actual);
    }
}

fn transition_to_carrying(
    commands: &mut Commands,
    entity: Entity,
    cell: IVec2,
    amount: u32,
    slots: &mut ResMut<SoftWorkSlots>,
) {
    slots.release(cell, IntentKind::Gather);
    commands
        .entity(entity)
        .remove::<ExtractProgress>()
        .remove::<GatherAssignment>();
    if amount > 0 {
        // No WorkerLoad for an empty extraction -- the worker
        // simply goes back to idle.
        commands.entity(entity).insert(WorkerLoad {
            kind: ResourceKind::Minerals,
            amount,
        });
    }
}

/// For each Worker that has a [`WorkerLoad`] but no destination
/// yet, find the nearest matching stockpile with free space and
/// start the delivery trip. Only [`ResourceKind::Minerals`] is
/// supported; multi-kind support is a follow-up.
#[allow(clippy::type_complexity)]
pub fn worker_gather_carry_assign_system(
    mut commands: Commands,
    workers: Query<
        (Entity, &Transform, &WorkerLoad),
        (
            With<Nanobot>,
            With<WorkerLoad>,
            Without<ReturningToStockpile>,
        ),
    >,
    stockpiles: Query<(Entity, &Stockpile, &Transform)>,
) {
    for (entity, transform, load) in &workers {
        let Some(stockpile_entity) =
            find_nearest_stockpile(load.kind, transform.translation.truncate(), &stockpiles)
        else {
            // No stockpile exists yet. The worker waits with the
            // load; a later tick that adds a stockpile will pick
            // it up.
            continue;
        };
        let Ok((_, _, stockpile_transform)) = stockpiles.get(stockpile_entity) else {
            continue;
        };
        commands.entity(entity).insert((
            ReturningToStockpile {
                stockpile: stockpile_entity,
            },
            DirectMovementComponent {
                xy: stockpile_transform.translation.truncate(),
            },
        ));
    }
}

/// Drop the worker's carry into the stockpile when the worker has
/// arrived. The movement system removes [`DirectMovementComponent`]
/// when the bot stops, which is the trigger this system waits for.
#[allow(clippy::type_complexity)]
pub fn worker_gather_delivery_system(
    mut commands: Commands,
    mut workers: Query<
        (Entity, &Transform, &mut WorkerLoad, &ReturningToStockpile),
        (
            With<Nanobot>,
            With<ReturningToStockpile>,
            Without<DirectMovementComponent>,
        ),
    >,
    mut stockpiles: Query<(&mut Stockpile, &Transform)>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, transform, mut load, returning) in &mut workers {
        let Ok((mut stockpile, stockpile_transform)) = stockpiles.get_mut(returning.stockpile)
        else {
            // Assigned stockpile is gone. Drop the load so the
            // worker can pick up new work.
            commands.entity(entity).remove::<ReturningToStockpile>();
            load.amount = 0;
            commands.entity(entity).remove::<WorkerLoad>();
            continue;
        };
        let distance = transform
            .translation
            .truncate()
            .distance(stockpile_transform.translation.truncate());
        if distance > stockpile.radius {
            continue;
        }
        if stockpile.free_space() < load.amount {
            // Chosen stockpile is too full. Release the
            // destination so the carry-assign system picks a
            // different one next tick; the load stays intact.
            commands.entity(entity).remove::<ReturningToStockpile>();
            continue;
        }
        let delivered = load.amount;
        stockpile.amount += delivered;
        ledger.add(stockpile.kind, delivered);
        load.amount = 0;
        commands
            .entity(entity)
            .remove::<ReturningToStockpile>()
            .remove::<WorkerLoad>();
    }
}

/// Plugin that wires the gather systems into the Update schedule.
/// The chain runs after `move_velocity_system` so the movement
/// system has already pruned arrived bots (which is the trigger the
/// arrive and delivery systems wait for).
///
/// Internal order: assignment -> source stockpile demand ->
/// arrive -> extract -> carry-assign -> delivery. The demand
/// system runs after assignment so it sees the current tick's
/// new assignments and plans a Source Stockpile on the same
/// tick the Worker is routed to the deposit; the planned
/// structure's claim system (in `PlannedStructurePlugin`,
/// registered after this one) then picks up the planned
/// structure for the same tick.
pub struct GatherPlugin;

impl Plugin for GatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                worker_gather_assignment_system,
                source_stockpile_demand_system,
                worker_gather_arrive_system,
                worker_gather_extract_system,
                worker_gather_carry_assign_system,
                worker_gather_delivery_system,
            )
                .chain()
                .after(crate::nanobot::move_velocity_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! (extraction, delivery, persistence, reactivation) are
    //! covered by `tests/gather_zone_behavior.rs`.

    use super::*;

    #[test]
    fn world_to_cell_finds_the_correct_intent_grid_cell() {
        // ZONE_BLOCK_SIZE is 512. Origin is world (0, 0) which is
        // grid cell (0, 0); small positive offsets are still
        // inside cell (0, 0); the first cell boundary at +512
        // moves into cell (1, 0). Negative offsets floor into
        // negative cells.
        assert_eq!(world_to_cell(Vec2::new(0.0, 0.0)), IVec2::new(0, 0));
        assert_eq!(world_to_cell(Vec2::new(100.0, 100.0)), IVec2::new(0, 0));
        assert_eq!(world_to_cell(Vec2::new(511.99, 0.0)), IVec2::new(0, 0));
        assert_eq!(world_to_cell(Vec2::new(512.0, 0.0)), IVec2::new(1, 0));
        assert_eq!(world_to_cell(Vec2::new(-1.0, 0.0)), IVec2::new(-1, 0));
        assert_eq!(world_to_cell(Vec2::new(100.0, -100.0)), IVec2::new(0, -1));
    }

    #[test]
    fn cell_overlaps_circle_center_inside_cell() {
        // Deposit center in the middle of cell (0, 0); even a
        // tiny radius overlaps. The cell rect is (0, 0) -
        // (512, 512), the center is (256, 256), so the distance
        // to the rect is 0 -- strictly less than any positive
        // radius.
        assert!(cell_overlaps_circle(
            IVec2::new(0, 0),
            Vec2::new(256.0, 256.0),
            0.001,
        ));
        assert!(cell_overlaps_circle(
            IVec2::new(0, 0),
            Vec2::new(256.0, 256.0),
            100.0,
        ));
    }

    #[test]
    fn cell_overlaps_circle_center_on_cell_boundary() {
        // A deposit center sitting exactly on the boundary
        // between cell (0, 0) and cell (1, 0) is on the edge of
        // both cells. The closest point on cell (0, 0)'s rect to
        // the boundary point is the boundary point itself, so
        // any positive radius overlaps. (This is the case the
        // issue calls out: "even when its center is not in the
        // same intent grid cell as the selected paint" should
        // also cover the boundary case -- the new logic is
        // strictly more permissive than exact cell membership.)
        let boundary = Vec2::new(512.0, 256.0);
        assert!(cell_overlaps_circle(IVec2::new(0, 0), boundary, 0.001));
        assert!(cell_overlaps_circle(IVec2::new(1, 0), boundary, 0.001));
    }

    #[test]
    fn cell_overlaps_circle_center_outside_radius_reaches_in() {
        // Deposit center is in cell (1, 0) but its radius is
        // large enough to reach into cell (0, 0). The closest
        // point on cell (0, 0)'s rect to (768, 256) is (512,
        // 256); the distance is 256. A radius of 300 makes the
        // deposit overlap both cells.
        let center_in_cell_one = Vec2::new(768.0, 256.0);
        assert!(!cell_overlaps_circle(
            IVec2::new(0, 0),
            center_in_cell_one,
            100.0,
        ));
        assert!(!cell_overlaps_circle(
            IVec2::new(0, 0),
            center_in_cell_one,
            255.0,
        ));
        assert!(cell_overlaps_circle(
            IVec2::new(0, 0),
            center_in_cell_one,
            256.0,
        ));
        assert!(cell_overlaps_circle(
            IVec2::new(0, 0),
            center_in_cell_one,
            300.0,
        ));
        // And the deposit's "home" cell still overlaps too.
        assert!(cell_overlaps_circle(
            IVec2::new(1, 0),
            center_in_cell_one,
            100.0,
        ));
    }

    #[test]
    fn cell_overlaps_circle_far_center_does_not_overlap() {
        // Deposit center is in cell (3, 0) with a small radius
        // that does not reach back to cell (0, 0). The closest
        // point on cell (0, 0)'s rect to (1792, 256) is (512,
        // 256); the distance is 1280, well beyond any reasonable
        // radius.
        let far_center = Vec2::new(1792.0, 256.0);
        assert!(!cell_overlaps_circle(IVec2::new(0, 0), far_center, 32.0));
        assert!(!cell_overlaps_circle(IVec2::new(0, 0), far_center, 1279.0));
        assert!(!cell_overlaps_circle(IVec2::new(0, 0), far_center, 1000.0));
    }

    #[test]
    fn cell_overlaps_circle_works_for_negative_cells() {
        // Same shape on the negative side of the origin. Cell
        // (-1, 0) rect is (-512, 0) to (0, 512). A deposit
        // center in cell (-2, 0) at (-768, 256) with a radius
        // that just reaches back into cell (-1, 0) should
        // overlap cell (-1, 0) but not cell (0, 0). The closest
        // point on cell (-1, 0)'s rect to (-768, 256) is
        // (-512, 256); the distance is 256, so radius >= 256
        // overlaps.
        let center = Vec2::new(-768.0, 256.0);
        assert!(!cell_overlaps_circle(IVec2::new(-1, 0), center, 100.0));
        assert!(cell_overlaps_circle(IVec2::new(-1, 0), center, 256.0));
        assert!(cell_overlaps_circle(IVec2::new(-1, 0), center, 300.0));
        // Cell (0, 0) rect is (0, 0) to (512, 512). The closest
        // point on that rect to (-768, 256) is (0, 256); the
        // distance is 768, so no reasonable radius reaches it.
        assert!(!cell_overlaps_circle(IVec2::new(0, 0), center, 700.0));
    }

    #[test]
    fn cell_overlaps_circle_zero_radius_is_point_test() {
        // A zero radius makes the test degenerate to a point:
        // the overlap is true only when the center is inside
        // the cell rect. A center on the cell boundary counts
        // as inside because the rect is half-open in the
        // overlap helper (we use clamp, so the boundary maps
        // to itself).
        assert!(cell_overlaps_circle(
            IVec2::new(0, 0),
            Vec2::new(256.0, 256.0),
            0.0,
        ));
        assert!(cell_overlaps_circle(
            IVec2::new(0, 0),
            Vec2::new(512.0, 256.0),
            0.0,
        ));
        assert!(!cell_overlaps_circle(
            IVec2::new(0, 0),
            Vec2::new(513.0, 256.0),
            0.0,
        ));
        assert!(!cell_overlaps_circle(
            IVec2::new(0, 0),
            Vec2::new(-1.0, 256.0),
            0.0,
        ));
    }
}
