//! Hauler behaviour and automatic stockpile creation.
//!
//! Haulers move large physical loads between logistics buffers:
//! source stockpiles, sink stockpiles, and terminal consumers
//! (production facilities / chargers). Deposits are worker-only
//! sources under the tiered logistics model; legacy manual hauler
//! assignments can still drain them defensively for tests.

use bevy::prelude::*;

use crate::ai::get_world_from_zone;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::{
    charge::Charger,
    components::{DirectMovementComponent, Nanobot, SwarmId, SwarmMember},
    gather::world_to_cell,
    logistics_leg::{pick_logistics_leg, HaulerContext, StockpileCandidate, TerminalCandidate},
    placement::BUILDING_FOOTPRINT_RADIUS,
    NanobotType, OwnerSwarm, ProductionFacility, STOP_THRESHOLD,
};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile, StockpileRole};

/// Maximum units a Hauler can carry in a single trip. The glossary is
/// explicit that Haulers carry "much more" than Workers; 40 is
/// deliberately ten times the worker cap so the gap is visible in the
/// swarm output and obvious in the test math.
pub const HAULER_CARRY_CAPACITY: u32 = 40;

/// Units a Hauler pulls from its source per `app.update()` tick.
/// 8 units/tick means a hauler fills the 40-unit load in 5 ticks;
/// large enough that the trip is short relative to the load but
/// small enough that the test can drive the simulation forward with
/// a handful of updates.
pub const HAULER_EXTRACT_PER_TICK: u32 = 8;

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

/// Marks a Hauler as biased by a Logistics Corridor paint. The
/// hauler first walks to `waypoint` (the corridor cell picked by
/// [`corridor_waypoint_between`]) and then continues to `target`
/// (the original source or sink the assignment chain gave the
/// hauler). The component is removed once the hauler reaches
/// `target`; while it is present, the waypoint system redirects
/// the hauler's [`DirectMovementComponent`] to keep the trip
/// inside the corridor.
///
/// A corridor alone never produces this component: the
/// assignment systems only insert it as a follow-up to a real
/// source/sink trip, preserving the "corridors bias paths, they
/// do not create jobs" contract.
#[derive(Debug, Component, Clone, Copy)]
pub struct HaulerCorridorWaypoint {
    pub waypoint: Vec2,
    pub target: Vec2,
}

/// Pushes the hauler toward `target` through a corridor waypoint
/// when the player painted a Logistics Corridor cell on the line
/// between the hauler's current position and its current
/// [`DirectMovementComponent`] target.
///
/// Two responsibilities live in one system because both depend on
/// the same per-tick check: assign a fresh waypoint when the
/// hauler starts a leg with a painted corridor, and progress an
/// existing waypoint through its `waypoint -> target` transition.
/// The system runs in the same chain as the rest of the hauler
/// stages so a freshly-assigned DMC is re-routed through the
/// corridor on the same tick.
///
/// A hauler with no painted corridor, or with a corridor that
/// is off the line, has no `HaulerCorridorWaypoint` inserted and
/// the original straight-line DMC stands. This is the path that
/// keeps the existing tests (and the player experience) green
/// when no corridor is painted.
#[allow(clippy::type_complexity)]
pub fn hauler_corridor_waypoint_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    haulers_in_transit: Query<
        (Entity, &Transform, &DirectMovementComponent),
        (
            With<Nanobot>,
            With<NanobotType>,
            With<DirectMovementComponent>,
            With<HaulerAssignment>,
            Without<HaulerCorridorWaypoint>,
        ),
    >,
    haulers_with_waypoint: Query<
        (Entity, &Transform, &HaulerCorridorWaypoint),
        (With<Nanobot>, With<HaulerCorridorWaypoint>),
    >,
) {
    for (entity, transform, dmc) in &haulers_in_transit {
        let pos = transform.translation.truncate();
        let Some(waypoint) = corridor_waypoint_between(pos, dmc.xy, &grid) else {
            continue;
        };
        // Same world position: no detour is needed. Skip the
        // waypoint so the hauler arrives normally.
        if (waypoint - dmc.xy).length() < 1.0 {
            continue;
        }
        commands.entity(entity).insert((
            HaulerCorridorWaypoint {
                waypoint,
                target: dmc.xy,
            },
            // Corridor waypoints are extent-less
            // destinations (issue #38 / ADR-0004): the
            // waypoint sits at a cell center, not on a
            // physical entity. The `0.0` sentinel falls
            // through to `STOP_THRESHOLD` in the movement
            // system, matching the pre-issue behaviour
            // for the corridor path.
            DirectMovementComponent {
                xy: waypoint,
                stop_radius: 0.0,
            },
        ));
    }

    for (entity, transform, waypoint) in &haulers_with_waypoint {
        let pos = transform.translation.truncate();
        let to_waypoint = pos.distance(waypoint.waypoint);
        let to_target = pos.distance(waypoint.target);

        if to_waypoint > STOP_THRESHOLD {
            // Still on the waypoint leg. If congestion caused the
            // movement progress timeout to strip the DMC, restore
            // it so the hauler does not sit forever with only the
            // waypoint marker.
            commands.entity(entity).insert(DirectMovementComponent {
                xy: waypoint.waypoint,
                stop_radius: 0.0,
            });
        } else if to_target > STOP_THRESHOLD {
            // At the waypoint, head to the original target.
            // The second leg's extent depends on whether
            // the hauler is going to the source or the
            // sink, so the assignment chain re-issues
            // the DMC with the right radius through the
            // carry-assign / hauler arrive systems. We
            // pass `0.0` (extent-less) here only as a
            // "keep moving" signal so the hauler reaches
            // the waypoint's neighbourhood; the
            // source/sink transition then sets the real
            // extent.
            commands.entity(entity).insert(DirectMovementComponent {
                xy: waypoint.target,
                stop_radius: 0.0,
            });
        } else {
            // At the target. The arrival hauler systems (arrive
            // or delivery) will fire on the same tick from the
            // DMC removal; clear the waypoint so the hauler is
            // ready for its next leg.
            commands.entity(entity).remove::<HaulerCorridorWaypoint>();
        }
    }
}

/// Find the highest-paint corridor cell on the straight line from
/// `start` to `end` and return its world center. Returns `None` when
/// no corridor is painted on any sampled cell on the line, or when
/// `start` and `end` coincide.
///
/// `corridor_waypoint_between` is the path-bias helper behind the
/// Logistics Corridor intent layer: when a hauler is travelling
/// between a source and a sink, the hauler systems use the cell
/// returned here as an intermediate waypoint so the hauler's path
/// follows the corridor. Without a corridor, the hauler falls back
/// to the straight-line `DirectMovementComponent` assigned by the
/// existing transport chain.
///
/// The function samples cells along the line at roughly one sample
/// per [`crate::ZONE_BLOCK_SIZE`] (capped at 64 samples) so a long
/// trip and a short trip both get a useful answer without scanning
/// the whole grid. Out-of-bounds cells are skipped silently. When
/// two sampled cells tie on paint strength, the one closer to
/// `start` wins because sampling iterates `start -> end`.
pub fn corridor_waypoint_between(start: Vec2, end: Vec2, grid: &IntentGrid) -> Option<Vec2> {
    let distance = start.distance(end);
    if distance < 1.0 {
        return None;
    }
    let n_samples = ((distance / crate::ZONE_BLOCK_SIZE).ceil() as usize).clamp(2, 64);
    let mut best: Option<(u8, IVec2)> = None;
    for i in 0..=n_samples {
        let t = i as f32 / n_samples as f32;
        let sample = start.lerp(end, t);
        let cell = world_to_cell(sample);
        let Some(intent_cell) = grid.cell(cell) else {
            continue;
        };
        let strength = intent_cell.strength(IntentKind::Corridor);
        if strength == 0 {
            continue;
        }
        if best.is_none_or(|(s, _)| strength > s) {
            best = Some((strength, cell));
        }
    }
    best.map(|(_, cell)| get_world_from_zone(cell))
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
        let Some(leg) = pick_logistics_leg(
            HaulerContext {
                pos: transform.translation.truncate(),
                swarm: swarm_member.0,
                kind: ResourceKind::Minerals,
                carry_capacity: HAULER_CARRY_CAPACITY,
            },
            &stockpile_candidates,
            &terminal_candidates,
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

        commands.entity(entity).insert((
            HaulerAssignment { source, sink },
            DirectMovementComponent {
                xy: source_pos,
                stop_radius: source_radius,
            },
        ));
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
        (Entity, &Transform, &HaulerAssignment),
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
    for (entity, transform, assignment) in &haulers {
        let (source_pos, source_radius) = if let Ok((d, t)) = deposits.get(assignment.source) {
            (t.translation.truncate(), d.radius)
        } else if let Ok((s, t)) = stockpiles.get(assignment.source) {
            (t.translation.truncate(), s.radius)
        } else if let Ok((c, t)) = chargers.get(assignment.source) {
            (t.translation.truncate(), c.radius)
        } else {
            // Source entity disappeared; drop the assignment and
            // let a later tick reassign.
            commands.entity(entity).remove::<HaulerAssignment>();
            continue;
        };
        if transform.translation.truncate().distance(source_pos) <= source_radius {
            commands
                .entity(entity)
                .insert(HaulerLoading { collected: 0 });
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
        (Entity, &Transform, &HaulerLoad, &HaulerAssignment),
        (
            With<Nanobot>,
            With<HaulerLoad>,
            Without<DirectMovementComponent>,
        ),
    >,
    stockpiles: Query<(&Stockpile, &Transform)>,
    facilities: Query<(&ProductionFacility, &Transform)>,
    chargers: Query<(&Charger, &Transform)>,
) {
    for (entity, transform, _load, assignment) in &haulers {
        let Some(sink) =
            sink_endpoint_snapshot(assignment.sink, &stockpiles, &facilities, &chargers)
        else {
            // Sink entity disappeared between assignment and
            // the carry phase. Drop the assignment so a later
            // tick re-evaluates; the load is kept so the hauler
            // can finish the trip if a future assignment points
            // back at the same kind of sink.
            commands.entity(entity).remove::<HaulerAssignment>();
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
        if transform.translation.truncate().distance(sink.pos) <= sink.radius {
            continue;
        }
        // Issue #38 / ADR-0004: the sink-leg DMC carries
        // the sink's physical extent so the movement
        // system stops on the sink's edge, matching the
        // delivery system's radius-based guard.
        commands.entity(entity).insert(DirectMovementComponent {
            xy: sink.pos,
            stop_radius: sink.radius,
        });
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
                    .remove::<HaulerLoad>();
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
                    .remove::<HaulerLoad>();
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
                hauler_assignment_system,
                hauler_arrive_source_system,
                hauler_load_system,
                hauler_carry_assign_system,
                hauler_delivery_system,
                hauler_corridor_waypoint_system,
            )
                .chain()
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
    use crate::intent::PAINT_STRENGTH_CAP;
    use crate::nanobot::gather::WORKER_CARRY_CAPACITY;

    #[test]
    fn hauler_carry_capacity_is_much_larger_than_worker_capacity() {
        // The glossary says haulers carry "much more" than
        // workers. 5x is the floor; 10x makes the gap obvious in
        // test math and swarm behaviour. A const block turns the
        // compile-time check into a real invariant and dodges
        // clippy's "assertion on a constant" lint.
        const { assert!(HAULER_CARRY_CAPACITY >= 5 * WORKER_CARRY_CAPACITY) };
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

    #[test]
    fn corridor_waypoint_between_returns_none_when_no_corridor() {
        // Tracer bullet for issue #9: the helper that finds a
        // corridor waypoint on a line must return None when no
        // corridor cells are painted. This pins the "corridors do
        // not create jobs" baseline: a missing corridor means a
        // straight-line hauler trip.
        let grid = IntentGrid::new(8, 8);
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(2_000.0, 0.0);
        assert!(super::corridor_waypoint_between(start, end, &grid).is_none());
    }

    #[test]
    fn corridor_waypoint_between_returns_painted_cell_center() {
        // Painting a single corridor cell on the line between
        // start and end must make the helper return that cell's
        // world center as the waypoint. This pins the "corridors
        // bias hauler paths" contract for the simplest case.
        let mut grid = IntentGrid::new(8, 8);
        // The line goes from (0, 0) to (2_000, 0). Cell (0, 0)
        // is the world origin's cell; cell (1, 0) is the first
        // cell past +512; the line passes through both.
        let painted = IVec2::new(1, 0);
        assert!(grid.paint(painted, IntentKind::Corridor, 4));
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(2_000.0, 0.0);
        let waypoint = super::corridor_waypoint_between(start, end, &grid)
            .expect("painted corridor must produce a waypoint");
        let painted_world = crate::ai::get_world_from_zone(painted);
        assert!(
            (waypoint - painted_world).length() < 1.0,
            "waypoint must be the painted cell's world center; got {waypoint:?}"
        );
    }

    #[test]
    fn corridor_waypoint_between_picks_highest_paint_strength() {
        // Two corridor cells on the line, one with low paint and
        // one with high paint. The hauler system must prefer the
        // high-paint cell so corridor Paint Strength can increase
        // path preference (acceptance criterion).
        let mut grid = IntentGrid::new(8, 8);
        let weak = IVec2::new(0, 0);
        let strong = IVec2::new(1, 0);
        assert!(grid.paint(weak, IntentKind::Corridor, 1));
        assert!(grid.paint(strong, IntentKind::Corridor, PAINT_STRENGTH_CAP));
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(2_000.0, 0.0);
        let waypoint = super::corridor_waypoint_between(start, end, &grid)
            .expect("painted corridor must produce a waypoint");
        let strong_world = crate::ai::get_world_from_zone(strong);
        assert!(
            (waypoint - strong_world).length() < 1.0,
            "waypoint must be the high-paint cell; got {waypoint:?}"
        );
    }

    #[test]
    fn corridor_waypoint_between_skips_out_of_bounds_cells() {
        // A small grid (3x3 spans -1..2 on both axes) cannot
        // hold a corridor waypoint for a line that exits the
        // grid. The helper must not crash and must not return a
        // world position outside the grid.
        let mut grid = IntentGrid::new(3, 3);
        // Paint the only corridor cell on the line that is still
        // in-bounds, so the test failure mode is unambiguous:
        // the helper must look at the line, not at the grid.
        let in_bounds = IVec2::new(1, 0);
        assert!(grid.paint(in_bounds, IntentKind::Corridor, 4));
        let start = Vec2::new(0.0, 0.0);
        let end = Vec2::new(20_000.0, 0.0);
        let wp = super::corridor_waypoint_between(start, end, &grid)
            .expect("in-bounds painted cell must produce a waypoint");
        let wp_cell = crate::nanobot::gather::world_to_cell(wp);
        assert!(grid.in_bounds(wp_cell), "waypoint must be in-bounds");
    }
}
