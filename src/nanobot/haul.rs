//! Hauler behaviour and automatic stockpile creation.
//!
//! Haulers move large physical loads between logistics buffers:
//! source stockpiles, sink stockpiles, and terminal consumers
//! (production facilities / chargers). Deposits are worker-only
//! sources under the tiered logistics model; legacy manual hauler
//! assignments can still drain them defensively for tests.

use bevy::prelude::*;

use crate::intent::IntentGrid;
use crate::nanobot::{
    charge::Charger,
    components::{DirectMovementComponent, Nanobot, SwarmId, SwarmMember},
    hauler_route_cost,
    logistics_leg::{
        pick_logistics_leg_with_cost, HaulerContext, StockpileCandidate, TerminalCandidate,
    },
    placement::BUILDING_FOOTPRINT_RADIUS,
    plan_hauler_route, NanobotType, OwnerSwarm, ProductionFacility, STOP_THRESHOLD,
};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole};

/// Maximum units a Hauler can carry in a single trip. The glossary is
/// explicit that Haulers carry "much more" than Workers; this cap is
/// deliberately five times the worker cap so the gap is visible in the
/// swarm output and obvious in the test math.
pub const HAULER_CARRY_CAPACITY: u32 = 20;

/// Units a Hauler pulls from its source per `app.update()` tick.
/// Four units/tick means a hauler fills the 20-unit load in 5 ticks;
/// large enough that the trip is short relative to the load but
/// small enough that the test can drive the simulation forward with
/// a handful of updates.
pub const HAULER_EXTRACT_PER_TICK: u32 = 4;

/// What a Hauler is currently carrying. Inserted when the hauler
/// finishes loading at the source, removed when the load is dropped
/// at the sink. Absence means the hauler is idle or doing other
/// work.
#[derive(Debug, Component, Clone, Copy)]
pub struct HaulerLoad {
    pub kind: ResourceKind,
    pub amount: u32,
}

/// Marks a Hauler as committed to a specific `(source, sink)` pair.
/// In normal tiered logistics, `source` is a non-empty stockpile and
/// `sink` is a sink stockpile or terminal consumer. Defensive legacy
/// paths still tolerate deposit sources for hand-seeded assignments.
/// Both are kept on the same component because the hauler commits to
/// the whole trip in the assignment system rather than picking the
/// sink at delivery time.
#[derive(Debug, Component, Clone, Copy)]
pub struct HaulerAssignment {
    pub source: Entity,
    pub sink: Entity,
}

/// Marker for a Hauler that is standing at its assigned source and
/// pulling resources. The `Without<HaulerLoad>` filter in
/// `hauler_load_system` makes the loading phase idempotent -- a
/// hauler only loads once per assignment, regardless of how many
/// ticks the load_system fires.
#[derive(Debug, Component, Default, Clone, Copy)]
pub struct HaulerLoading {
    pub collected: u32,
}

/// Stable route for one hauler Logistics Leg.
#[derive(Debug, Component, Clone)]
pub struct HaulerRoute {
    pub waypoints: Vec<Vec2>,
    pub current: usize,
    pub final_stop_radius: f32,
}

impl HaulerRoute {
    pub fn new(waypoints: Vec<Vec2>, final_stop_radius: f32) -> Self {
        Self {
            waypoints,
            current: 0,
            final_stop_radius,
        }
    }

    fn current_waypoint(&self) -> Option<Vec2> {
        self.waypoints.get(self.current).copied()
    }

    fn current_stop_radius(&self) -> f32 {
        if self.current + 1 == self.waypoints.len() {
            self.final_stop_radius
        } else {
            0.0
        }
    }

    fn current_movement(&self) -> Option<DirectMovementComponent> {
        self.current_waypoint().map(|xy| DirectMovementComponent {
            xy,
            stop_radius: self.current_stop_radius(),
        })
    }
}

/// Follow a stable route by issuing the current waypoint as direct movement.
pub fn hauler_route_follow_system(
    mut commands: Commands,
    mut haulers: Query<(
        Entity,
        &Transform,
        &mut HaulerRoute,
        Option<&DirectMovementComponent>,
    )>,
) {
    for (entity, transform, mut route, dmc) in &mut haulers {
        if route.waypoints.is_empty() {
            commands.entity(entity).remove::<HaulerRoute>();
            continue;
        }

        let pos = transform.translation.truncate();
        loop {
            let Some(waypoint) = route.current_waypoint() else {
                commands.entity(entity).remove::<HaulerRoute>();
                break;
            };
            let stop_radius = route.current_stop_radius();
            let threshold = if stop_radius > 0.0 {
                stop_radius.max(STOP_THRESHOLD)
            } else {
                STOP_THRESHOLD
            };
            if pos.distance(waypoint) > threshold {
                if dmc.is_none_or(|dmc| {
                    (dmc.xy - waypoint).length() > 1.0
                        || (dmc.stop_radius - stop_radius).abs() > f32::EPSILON
                }) {
                    commands.entity(entity).insert(DirectMovementComponent {
                        xy: waypoint,
                        stop_radius,
                    });
                }
                break;
            }
            route.current += 1;
        }
    }
}

fn route_waypoints_or_direct(
    start: Vec2,
    end: Vec2,
    grid: &IntentGrid,
    swarm: SwarmId,
) -> Vec<Vec2> {
    plan_hauler_route(start, end, grid, swarm)
        .map(|route| route.waypoints)
        .unwrap_or_else(|| vec![end])
}

pub(crate) fn planned_route_movement(
    start: Vec2,
    end: Vec2,
    grid: &IntentGrid,
    swarm: SwarmId,
    final_stop_radius: f32,
) -> (HaulerRoute, DirectMovementComponent) {
    let route = HaulerRoute::new(
        route_waypoints_or_direct(start, end, grid, swarm),
        final_stop_radius,
    );
    let movement = route.current_movement().unwrap_or(DirectMovementComponent {
        xy: end,
        stop_radius: final_stop_radius,
    });
    (route, movement)
}

/// Convert an optional [`OwnerSwarm`] marker into the concrete [`SwarmId`]
/// used by the pure Logistics Leg picker. A broken owner reference
/// makes the candidate unusable, matching the old `owner_matches`
/// behaviour.
fn candidate_owner(
    owner: Option<&OwnerSwarm>,
    swarms: &Query<&SwarmId>,
) -> Option<Option<SwarmId>> {
    match owner {
        None => Some(None),
        Some(OwnerSwarm(owner_entity)) => swarms.get(*owner_entity).ok().copied().map(Some),
    }
}

/// World position of a hauler source stockpile. A hauler source
/// is always a stockpile under the tiered model, so this is a
/// plain component lookup.
#[allow(clippy::type_complexity)]
fn stockpile_pos(
    entity: Entity,
    stockpiles: &Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
) -> Option<Vec2> {
    stockpiles
        .get(entity)
        .ok()
        .map(|(_, _, t, _, _)| t.translation.truncate())
}

/// Physical extent of a hauler source stockpile, used as the
/// arrival stop radius so the movement system halts on the
/// stockpile's edge (matching the arrive-source guard).
#[allow(clippy::type_complexity)]
fn stockpile_radius_of(
    entity: Entity,
    stockpiles: &Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
) -> f32 {
    stockpiles
        .get(entity)
        .map(|(_, s, _, _, _)| s.radius)
        .unwrap_or(0.0)
}

/// For each idle Hauler with no in-flight transport work, pick a
/// `(source, sink)` pair from the resource economy and head to the
/// source. The hauler keeps a single [`HaulerAssignment`] for the
/// whole trip so the carry-to-sink step does not need to re-select
/// the sink from scratch.
#[allow(clippy::type_complexity)]
pub fn hauler_assignment_system(
    mut commands: Commands,
    haulers: Query<
        (Entity, &Transform, &NanobotType, &SwarmMember),
        (
            With<Nanobot>,
            With<NanobotType>,
            Without<HaulerAssignment>,
            Without<HaulerLoad>,
            Without<HaulerLoading>,
            Without<DirectMovementComponent>,
        ),
    >,
    stockpiles: Query<(
        Entity,
        &Stockpile,
        &Transform,
        Option<&StockpileRole>,
        Option<&OwnerSwarm>,
    )>,
    facilities: Query<(Entity, &ProductionFacility, &Transform, Option<&OwnerSwarm>)>,
    chargers: Query<(Entity, &Charger, &Transform, Option<&OwnerSwarm>)>,
    swarms: Query<&SwarmId>,
    grid: Res<IntentGrid>,
) {
    let stockpile_candidates: Vec<StockpileCandidate> = stockpiles
        .iter()
        .filter_map(|(entity, stockpile, transform, role, owner)| {
            let owner = candidate_owner(owner, &swarms)?;
            Some(StockpileCandidate {
                entity,
                pos: transform.translation.truncate(),
                kind: stockpile.kind,
                role: role.copied().unwrap_or(StockpileRole::Source),
                amount: stockpile.amount,
                free_space: stockpile.free_space(),
                owner,
            })
        })
        .collect();
    let mut terminal_candidates: Vec<TerminalCandidate> = facilities
        .iter()
        .filter_map(|(entity, facility, transform, owner)| {
            let owner = candidate_owner(owner, &swarms)?;
            Some(TerminalCandidate::Facility {
                entity,
                pos: transform.translation.truncate(),
                kind: facility.input_kind,
                free_space: facility.input_free_space(),
                owner,
            })
        })
        .collect();
    terminal_candidates.extend(chargers.iter().filter_map(
        |(entity, charger, transform, owner)| {
            let owner = candidate_owner(owner, &swarms)?;
            Some(TerminalCandidate::Charger {
                entity,
                pos: transform.translation.truncate(),
                kind: charger.kind,
                free_space: charger.free_space(),
                owner,
            })
        },
    ));

    for (entity, transform, nanobot_type, swarm_member) in &haulers {
        if *nanobot_type != NanobotType::Hauler {
            continue;
        }
        let hauler_pos = transform.translation.truncate();
        let swarm = swarm_member.0;
        let Some(leg) = pick_logistics_leg_with_cost(
            HaulerContext {
                pos: hauler_pos,
                swarm,
                kind: ResourceKind::Minerals,
                carry_capacity: HAULER_CARRY_CAPACITY,
            },
            &stockpile_candidates,
            &terminal_candidates,
            |from, to| hauler_route_cost(from, to, &grid, swarm),
        ) else {
            continue;
        };
        let source = leg.source;
        let sink = leg.sink;
        let Some(source_pos) = stockpile_pos(source, &stockpiles) else {
            continue;
        };

        // Source-side stop radius: the source stockpile's own
        // physical extent. A hauler source is always a stockpile
        // under the tiered model (deposits are a worker-only
        // source), so the lookup never falls through. Issue #38
        // / ADR-0004: same extent as the hauler-arrive-source
        // guard so the movement system and the arrive system
        // stop on the same edge.
        let source_radius = stockpile_radius_of(source, &stockpiles);

        let (route, movement) =
            planned_route_movement(hauler_pos, source_pos, &grid, swarm, source_radius);

        commands
            .entity(entity)
            .insert((HaulerAssignment { source, sink }, route, movement));
    }
}

/// Detect a hauler that has arrived at its assigned source and
/// start the loading phase. The arrival threshold is the source's
/// own radius (deposit or stockpile), matching the gather chain.
/// The `Without<HaulerLoading>` filter makes arrival idempotent; the
/// `Without<HaulerLoad>` filter keeps a Carrying hauler from being
/// re-loaded when it happens to be at the source between trips.
#[allow(clippy::type_complexity)]
pub fn hauler_arrive_source_system(
    mut commands: Commands,
    haulers: Query<
        (Entity, &Transform, &HaulerAssignment, Option<&HaulerRoute>),
        (
            With<Nanobot>,
            With<HaulerAssignment>,
            Without<DirectMovementComponent>,
            Without<HaulerLoading>,
            Without<HaulerLoad>,
        ),
    >,
    deposits: Query<(&ResourceDeposit, &Transform)>,
    stockpiles: Query<(&Stockpile, &Transform)>,
    chargers: Query<(&Charger, &Transform)>,
) {
    for (entity, transform, assignment, route) in &haulers {
        let (source_pos, source_radius) = if let Ok((d, t)) = deposits.get(assignment.source) {
            (t.translation.truncate(), d.radius)
        } else if let Ok((s, t)) = stockpiles.get(assignment.source) {
            (t.translation.truncate(), s.radius)
        } else if let Ok((c, t)) = chargers.get(assignment.source) {
            (t.translation.truncate(), c.radius)
        } else {
            // Source entity disappeared; drop the assignment and
            // let a later tick reassign.
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<HaulerRoute>();
            continue;
        };
        if transform.translation.truncate().distance(source_pos) <= source_radius {
            commands
                .entity(entity)
                .insert(HaulerLoading { collected: 0 })
                .remove::<HaulerRoute>();
        } else if route.is_some() {
            // The route follower owns movement restoration while a
            // route is active.
        } else {
            // `ProgressChecker` can remove `DirectMovementComponent`
            // before true arrival when congestion leaves the hauler
            // below the progress threshold. Keep the source-side
            // commitment alive and restore movement instead of
            // marooning the hauler with `HaulerAssignment` and no
            // velocity.
            commands.entity(entity).insert(DirectMovementComponent {
                xy: source_pos,
                stop_radius: source_radius,
            });
        }
    }
}

/// Drain `HAULER_EXTRACT_PER_TICK` units from the assigned source
/// every tick while the hauler is at the source and the load is
/// not full. When the load is full or the source empties (or
/// disappears), transition the hauler to Carrying.
#[allow(clippy::type_complexity)]
pub fn hauler_load_system(
    mut commands: Commands,
    mut haulers: Query<
        (Entity, &mut HaulerLoading, &HaulerAssignment),
        (With<Nanobot>, With<HaulerLoading>),
    >,
    mut deposits: Query<&mut ResourceDeposit>,
    mut source_stockpiles: Query<&mut Stockpile>,
    source_chargers: Query<&mut Charger>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, mut loading, assignment) in &mut haulers {
        if loading.collected >= HAULER_CARRY_CAPACITY {
            transition_to_carrying(&mut commands, entity, loading.collected);
            continue;
        }

        if let Ok(mut deposit) = deposits.get_mut(assignment.source) {
            if deposit.amount == 0 {
                transition_to_carrying(&mut commands, entity, loading.collected);
                continue;
            }
            let can_still_carry = HAULER_CARRY_CAPACITY - loading.collected;
            let actual = HAULER_EXTRACT_PER_TICK
                .min(deposit.amount)
                .min(can_still_carry);
            loading.collected += actual;
            deposit.amount -= actual;
            ledger.remove(deposit.kind, actual);
            continue;
        }

        if let Ok(mut stockpile) = source_stockpiles.get_mut(assignment.source) {
            if stockpile.amount == 0 {
                transition_to_carrying(&mut commands, entity, loading.collected);
                continue;
            }
            let can_still_carry = HAULER_CARRY_CAPACITY - loading.collected;
            let actual = HAULER_EXTRACT_PER_TICK
                .min(stockpile.amount)
                .min(can_still_carry);
            loading.collected += actual;
            stockpile.amount -= actual;
            ledger.remove(stockpile.kind, actual);
            continue;
        }

        if source_chargers.get(assignment.source).is_ok() {
            // Chargers are not a hauler source: the logistics
            // contract is "haulers bring material TO the
            // charger", not "haulers extract FROM the charger".
            // If a future assignment points at a charger as a
            // source, drop the assignment with no load so the
            // hauler re-picks work on the next tick.
            transition_to_carrying(&mut commands, entity, 0);
            continue;
        }

        // Source entity disappeared; the partial load is still
        // useful so we still transition to Carrying.
        transition_to_carrying(&mut commands, entity, loading.collected);
    }
}

fn transition_to_carrying(commands: &mut Commands, entity: Entity, amount: u32) {
    commands.entity(entity).remove::<HaulerLoading>();
    if amount > 0 {
        commands.entity(entity).insert(HaulerLoad {
            kind: ResourceKind::Minerals,
            amount,
        });
    } else {
        // Nothing to carry; drop the assignment and let the
        // hauler pick new work next tick.
        commands.entity(entity).remove::<HaulerAssignment>();
    }
}

/// Snapshot of a resource sink endpoint for the carry leg.
/// Stockpiles, production facilities, and chargers expose
/// different component shapes, but the movement system only needs
/// the world position and physical reach of the chosen sink.
#[derive(Debug, Clone, Copy)]
struct SinkEndpointSnapshot {
    pos: Vec2,
    radius: f32,
}

/// Adapt a sink entity into the common endpoint shape used by the
/// hauler carry leg. Facilities do not carry their own radius field,
/// so their endpoint extent is the building footprint.
fn sink_endpoint_snapshot(
    entity: Entity,
    stockpiles: &Query<(&Stockpile, &Transform)>,
    facilities: &Query<(&ProductionFacility, &Transform)>,
    chargers: &Query<(&Charger, &Transform)>,
) -> Option<SinkEndpointSnapshot> {
    if let Ok((stockpile, transform)) = stockpiles.get(entity) {
        Some(SinkEndpointSnapshot {
            pos: transform.translation.truncate(),
            radius: stockpile.radius,
        })
    } else if let Ok((_, transform)) = facilities.get(entity) {
        Some(SinkEndpointSnapshot {
            pos: transform.translation.truncate(),
            radius: BUILDING_FOOTPRINT_RADIUS,
        })
    } else if let Ok((charger, transform)) = chargers.get(entity) {
        Some(SinkEndpointSnapshot {
            pos: transform.translation.truncate(),
            radius: charger.radius,
        })
    } else {
        None
    }
}

/// For each Hauler that has a [`HaulerLoad`] but no destination yet,
/// head to the sink recorded on the [`HaulerAssignment`]. The sink
/// was chosen at assignment time so the hauler does not need to
/// re-evaluate.
#[allow(clippy::type_complexity)]
pub fn hauler_carry_assign_system(
    mut commands: Commands,
    haulers: Query<
        (
            Entity,
            &Transform,
            &HaulerLoad,
            &HaulerAssignment,
            &SwarmMember,
            Option<&HaulerRoute>,
        ),
        (
            With<Nanobot>,
            With<HaulerLoad>,
            Without<DirectMovementComponent>,
        ),
    >,
    stockpiles: Query<(&Stockpile, &Transform)>,
    facilities: Query<(&ProductionFacility, &Transform)>,
    chargers: Query<(&Charger, &Transform)>,
    grid: Res<IntentGrid>,
) {
    for (entity, transform, _load, assignment, swarm_member, route) in &haulers {
        let Some(sink) =
            sink_endpoint_snapshot(assignment.sink, &stockpiles, &facilities, &chargers)
        else {
            // Sink entity disappeared between assignment and
            // the carry phase. Drop the assignment so a later
            // tick re-evaluates; the load is kept so the hauler
            // can finish the trip if a future assignment points
            // back at the same kind of sink.
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<HaulerRoute>();
            continue;
        };
        // If the hauler is already at the sink, the delivery
        // system must fire before we re-target. Inserting a
        // fresh DirectMovementComponent here would clear the
        // arrival signal and starve the delivery system, leaving
        // the hauler stuck in an infinite carry/loop cycle.
        //
        // Issue #38 / ADR-0004: the proximity check now
        // matches the DMC's `stop_radius` (the sink's own
        // radius). Using `STOP_THRESHOLD` here would race
        // with the movement system: the movement system
        // stops at `max(stop_radius, STOP_THRESHOLD)`, then
        // the carry-assign re-inserts a DMC because the
        // hauler is between `STOP_THRESHOLD` and
        // `stop_radius`, then delivery's
        // `Without<DirectMovementComponent>` filter rejects
        // the arrival. Using the same radius the movement
        // system uses breaks the loop.
        let hauler_pos = transform.translation.truncate();
        if hauler_pos.distance(sink.pos) <= sink.radius || route.is_some() {
            continue;
        }
        let (route, movement) =
            planned_route_movement(hauler_pos, sink.pos, &grid, swarm_member.0, sink.radius);
        commands.entity(entity).insert((route, movement));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryResult {
    Delivered,
    TooFar,
    Full,
    Missing,
}

fn deliver_to_sink(
    sink: Entity,
    hauler_pos: Vec2,
    amount: u32,
    stockpiles: &mut Query<(&mut Stockpile, &Transform)>,
    facilities: &mut Query<(&mut ProductionFacility, &Transform)>,
    chargers: &mut Query<(&mut Charger, &Transform)>,
    ledger: &mut ResourceLedger,
) -> DeliveryResult {
    if let Ok((mut stockpile, transform)) = stockpiles.get_mut(sink) {
        if hauler_pos.distance(transform.translation.truncate()) > stockpile.radius {
            return DeliveryResult::TooFar;
        }
        if stockpile.free_space() < amount {
            return DeliveryResult::Full;
        }
        stockpile.amount += amount;
        ledger.add(stockpile.kind, amount);
        DeliveryResult::Delivered
    } else if let Ok((mut facility, transform)) = facilities.get_mut(sink) {
        // Leg 3 delivery into a facility's own input hopper. The
        // hopper is the only buffer production consumes from, so
        // this is the moment material actually reaches production.
        if hauler_pos.distance(transform.translation.truncate()) > BUILDING_FOOTPRINT_RADIUS {
            return DeliveryResult::TooFar;
        }
        if facility.input_free_space() < amount {
            return DeliveryResult::Full;
        }
        facility.input_amount += amount;
        ledger.add(facility.input_kind, amount);
        DeliveryResult::Delivered
    } else if let Ok((mut charger, transform)) = chargers.get_mut(sink) {
        if hauler_pos.distance(transform.translation.truncate()) > charger.radius {
            return DeliveryResult::TooFar;
        }
        if charger.free_space() < amount {
            return DeliveryResult::Full;
        }
        charger.amount += amount;
        ledger.add(charger.kind, amount);
        DeliveryResult::Delivered
    } else {
        DeliveryResult::Missing
    }
}

/// Drop the hauler's carry into the assigned sink when the hauler
/// has arrived. The arrival trigger is the movement system removing
/// the [`DirectMovementComponent`], which is the same signal the
/// worker delivery system uses.
#[allow(clippy::type_complexity)]
pub fn hauler_delivery_system(
    mut commands: Commands,
    mut haulers: Query<
        (Entity, &Transform, &mut HaulerLoad, &HaulerAssignment),
        (
            With<Nanobot>,
            With<HaulerLoad>,
            With<HaulerAssignment>,
            Without<DirectMovementComponent>,
        ),
    >,
    mut stockpiles: Query<(&mut Stockpile, &Transform)>,
    mut facilities: Query<(&mut ProductionFacility, &Transform)>,
    mut chargers: Query<(&mut Charger, &Transform)>,
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, transform, mut load, assignment) in &mut haulers {
        match deliver_to_sink(
            assignment.sink,
            transform.translation.truncate(),
            load.amount,
            &mut stockpiles,
            &mut facilities,
            &mut chargers,
            &mut ledger,
        ) {
            DeliveryResult::Delivered => {
                load.amount = 0;
                commands
                    .entity(entity)
                    .remove::<HaulerAssignment>()
                    .remove::<HaulerLoad>()
                    .remove::<HaulerRoute>();
            }
            DeliveryResult::TooFar | DeliveryResult::Full => {
                // Too far: wait for movement/assignment to restore
                // the arrival path. Full: keep waiting at the sink;
                // redirecting is a known first-implementation
                // limitation.
            }
            DeliveryResult::Missing => {
                // Assigned sink is gone. Drop the load so the hauler
                // can pick new work; the assignment is removed too so
                // the assignment system can re-evaluate on the next
                // tick.
                commands
                    .entity(entity)
                    .remove::<HaulerAssignment>()
                    .remove::<HaulerLoad>()
                    .remove::<HaulerRoute>();
            }
        }
    }
}

/// Plugin that wires the hauler systems into the Update
/// schedule. The chain runs after `move_velocity_system` so the
/// movement system has already pruned arrived bots (which is the
/// trigger the arrive and delivery systems wait for).
///
/// Note: the previous "instant stockpile" auto-creation system
/// (issue #8's `stockpile_auto_creation_system`) was removed in
/// issue #26. Sink Stockpiles now emerge through the planned
/// structure lifecycle in [`PlannedStructurePlugin`], where a
/// Build-painted cell plans a `PlannedKind::SinkStockpile` that
/// a Worker builds into a completed `Stockpile` stamped with
/// [`crate::resources::StockpileRole::Sink`]. Source Stockpiles
/// follow the same lifecycle but live in Gather cells (see
/// [`crate::nanobot::gather::source_stockpile_demand_system`]).
/// There is no longer a path that spawns a completed `Stockpile`
/// directly from Build paint.
pub struct HaulPlugin;

impl Plugin for HaulPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                hauler_arrive_source_system,
                hauler_load_system,
                hauler_carry_assign_system,
                hauler_delivery_system,
                hauler_route_follow_system,
            )
                .chain()
                .after(crate::nanobot::RegionalAllocationSet::Acquire)
                .after(crate::nanobot::move_velocity_system),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Pure-helper unit tests. The end-to-end contracts
    //! (transport, capacity, auto-creation) are covered by
    //! `tests/stockpile_and_haul_behavior.rs`.

    use super::*;
    use crate::nanobot::{gather::WORKER_CARRY_CAPACITY, planned::DEFAULT_STOCKPILE_CAPACITY};

    #[test]
    fn hauler_carry_capacity_is_much_larger_than_worker_capacity() {
        // The glossary says haulers carry "much more" than
        // workers. 5x is the floor that keeps the gap visible in
        // test math and swarm behaviour. A const block turns the
        // compile-time check into a real invariant and dodges
        // clippy's "assertion on a constant" lint.
        const { assert!(HAULER_CARRY_CAPACITY >= 5 * WORKER_CARRY_CAPACITY) };
    }

    #[test]
    fn hauler_carry_capacity_is_one_tenth_of_stockpile_capacity() {
        // One full hauler load is one tenth of a completed
        // Source or Sink Stockpile buffer.
        const { assert!(HAULER_CARRY_CAPACITY * 10 == DEFAULT_STOCKPILE_CAPACITY) };
    }

    #[test]
    fn hauler_extract_per_tick_divides_capacity() {
        // The hauler's load fills in a small whole number of
        // ticks. This keeps test math simple and avoids a
        // "stuck at the source for an awkward number of ticks"
        // pattern. Const block keeps the check compile-time so
        // a future tuning pass that breaks the invariant fails
        // the build, not just a test run.
        const { assert!(HAULER_CARRY_CAPACITY.is_multiple_of(HAULER_EXTRACT_PER_TICK)) };
    }
}
