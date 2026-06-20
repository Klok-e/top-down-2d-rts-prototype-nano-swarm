//! Hauler behaviour and automatic stockpile creation.
//!
//! Issue #8 contract: haulers move large physical loads between sources
//! (deposits and non-empty stockpiles) and sinks (stockpiles with free
//! space). Stockpiles emerge automatically from sustained Gather / Build
//! demand so the swarm always has a drop-off point near work.

use bevy::prelude::*;

use crate::ai::get_world_from_zone;
use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::{
    components::{DirectMovementComponent, Nanobot},
    gather::world_to_cell,
    NanobotType, STOP_THRESHOLD,
};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};

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
/// `source` is the deposit or non-empty stockpile the hauler will pull
/// from; `sink` is the non-full stockpile the hauler will drop the
/// load into. Both are kept on the same component because the hauler
/// commits to the whole trip in the assignment system rather than
/// picking the sink at delivery time.
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

/// Default kind, capacity, and radius for an auto-created stockpile.
/// Matches the manual stockpile spawned in `lib.rs` so the swarm
/// cannot tell the two apart.
pub const AUTO_STOCKPILE_KIND: ResourceKind = ResourceKind::Minerals;
pub const AUTO_STOCKPILE_CAPACITY: u32 = 1000;
pub const AUTO_STOCKPILE_RADIUS: f32 = 64.0;

/// Find the (source, sink) pair a hauler should commit to.
///
/// The selection heuristic is the simple greedy version: pick the
/// nearest source with matching kind and free resources, then pick
/// the nearest sink with matching kind and free space relative to
/// that source. The greedy choice is good enough for the first
/// implementation and keeps the system predictable for tests.
///
/// `worker_pos` is the world position of the hauler. `kind` is the
/// resource kind the hauler currently wants to transport (only
/// [`ResourceKind::Minerals`] is supported for now; multi-kind
/// support is a follow-up issue).
pub fn find_transport_pair(
    worker_pos: Vec2,
    kind: ResourceKind,
    deposits: &Query<(Entity, &ResourceDeposit, &Transform)>,
    stockpiles: &Query<(Entity, &Stockpile, &Transform)>,
) -> Option<(Entity, Entity)> {
    let source = find_nearest_source(worker_pos, kind, deposits, stockpiles)?;
    let source_pos = source_transform(source, deposits, stockpiles)?;
    let sink = find_nearest_sink(source, source_pos, kind, stockpiles)?;
    Some((source, sink))
}

fn find_nearest_source(
    worker_pos: Vec2,
    kind: ResourceKind,
    deposits: &Query<(Entity, &ResourceDeposit, &Transform)>,
    stockpiles: &Query<(Entity, &Stockpile, &Transform)>,
) -> Option<Entity> {
    let mut best_deposit: Option<(f32, Entity)> = None;
    for (entity, deposit, transform) in deposits.iter() {
        if deposit.kind != kind || deposit.amount == 0 {
            continue;
        }
        let d = worker_pos.distance(transform.translation.truncate());
        if best_deposit.is_none_or(|(bd, _)| d < bd) {
            best_deposit = Some((d, entity));
        }
    }
    let mut best_stockpile: Option<(f32, Entity)> = None;
    for (entity, stockpile, transform) in stockpiles.iter() {
        if stockpile.kind != kind || stockpile.amount == 0 {
            continue;
        }
        let d = worker_pos.distance(transform.translation.truncate());
        if best_stockpile.is_none_or(|(bd, _)| d < bd) {
            best_stockpile = Some((d, entity));
        }
    }
    match (best_deposit, best_stockpile) {
        (Some((d1, e1)), Some((d2, e2))) => {
            if d1 <= d2 {
                Some(e1)
            } else {
                Some(e2)
            }
        }
        (Some((_, e)), None) => Some(e),
        (None, Some((_, e))) => Some(e),
        (None, None) => None,
    }
}

fn source_transform(
    entity: Entity,
    deposits: &Query<(Entity, &ResourceDeposit, &Transform)>,
    stockpiles: &Query<(Entity, &Stockpile, &Transform)>,
) -> Option<Vec2> {
    if let Ok((_, _, t)) = deposits.get(entity) {
        Some(t.translation.truncate())
    } else if let Ok((_, _, t)) = stockpiles.get(entity) {
        Some(t.translation.truncate())
    } else {
        None
    }
}

fn find_nearest_sink(
    source: Entity,
    source_pos: Vec2,
    kind: ResourceKind,
    stockpiles: &Query<(Entity, &Stockpile, &Transform)>,
) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for (entity, stockpile, transform) in stockpiles.iter() {
        if stockpile.kind != kind || stockpile.free_space() == 0 || entity == source {
            continue;
        }
        let d = source_pos.distance(transform.translation.truncate());
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, entity));
        }
    }
    best.map(|(_, e)| e)
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
        (Entity, &Transform, &NanobotType),
        (
            With<Nanobot>,
            With<NanobotType>,
            Without<HaulerAssignment>,
            Without<HaulerLoad>,
            Without<HaulerLoading>,
            Without<DirectMovementComponent>,
        ),
    >,
    deposits: Query<(Entity, &ResourceDeposit, &Transform)>,
    stockpiles: Query<(Entity, &Stockpile, &Transform)>,
) {
    for (entity, transform, nanobot_type) in &haulers {
        if *nanobot_type != NanobotType::Hauler {
            continue;
        }
        let worker_pos = transform.translation.truncate();

        let Some((source, sink)) =
            find_transport_pair(worker_pos, ResourceKind::Minerals, &deposits, &stockpiles)
        else {
            continue;
        };
        let Some(source_pos) = source_transform(source, &deposits, &stockpiles) else {
            continue;
        };

        commands.entity(entity).insert((
            HaulerAssignment { source, sink },
            DirectMovementComponent { xy: source_pos },
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
) {
    for (entity, transform, assignment) in &haulers {
        let (source_pos, source_radius) = if let Ok((d, t)) = deposits.get(assignment.source) {
            (t.translation.truncate(), d.radius)
        } else if let Ok((s, t)) = stockpiles.get(assignment.source) {
            (t.translation.truncate(), s.radius)
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
    stockpiles: Query<&Transform, With<Stockpile>>,
) {
    for (entity, transform, _load, assignment) in &haulers {
        let Ok(sink_transform) = stockpiles.get(assignment.sink) else {
            continue;
        };
        // If the hauler is already at the sink, the delivery
        // system must fire before we re-target. Inserting a
        // fresh DirectMovementComponent here would clear the
        // arrival signal and starve the delivery system, leaving
        // the hauler stuck in an infinite carry/loop cycle.
        if transform
            .translation
            .truncate()
            .distance(sink_transform.translation.truncate())
            <= STOP_THRESHOLD
        {
            continue;
        }
        commands.entity(entity).insert(DirectMovementComponent {
            xy: sink_transform.translation.truncate(),
        });
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
    mut ledger: ResMut<ResourceLedger>,
) {
    for (entity, transform, mut load, assignment) in &mut haulers {
        let Ok((mut sink, sink_transform)) = stockpiles.get_mut(assignment.sink) else {
            // Assigned sink is gone. Drop the load so the hauler
            // can pick new work; the assignment is removed too so
            // the assignment system can re-evaluate on the next
            // tick.
            commands
                .entity(entity)
                .remove::<HaulerAssignment>()
                .remove::<HaulerLoad>();
            continue;
        };
        if transform
            .translation
            .truncate()
            .distance(sink_transform.translation.truncate())
            > sink.radius
        {
            continue;
        }
        if sink.free_space() < load.amount {
            // Sink too full. The carry-assign system reuses the
            // same assignment.sink, so the hauler cannot redirect
            // to a different sink. The hauler waits at the sink
            // until it is freed -- a known limitation for the
            // first implementation, addressed in a follow-up.
            continue;
        }
        let delivered = load.amount;
        sink.amount += delivered;
        ledger.add(sink.kind, delivered);
        load.amount = 0;
        commands
            .entity(entity)
            .remove::<HaulerAssignment>()
            .remove::<HaulerLoad>();
    }
}

/// Walk the [`IntentGrid`] and spawn a new [`Stockpile`] in any
/// Gather or Build cell that has paint but no stockpile. The
/// acceptance criterion is "stockpiles emerge automatically from
/// sustained gather/build demand"; painting the cell is the
/// player's expression of that demand.
///
/// The system reads the current set of stockpile positions every
/// tick rather than caching it, so a destroyed stockpile is
/// automatically respawned on the next tick if the demand is
/// still painted.
pub fn stockpile_auto_creation_system(
    mut commands: Commands,
    grid: Res<IntentGrid>,
    stockpiles: Query<&Transform, With<Stockpile>>,
) {
    let mut cells_with_stockpile: std::collections::HashSet<IVec2> =
        std::collections::HashSet::new();
    for transform in &stockpiles {
        cells_with_stockpile.insert(world_to_cell(transform.translation.truncate()));
    }
    for (cell, intent_cell) in grid.iter_cells() {
        if intent_cell.is_empty() {
            continue;
        }
        if !intent_cell.has(IntentKind::Gather) && !intent_cell.has(IntentKind::Build) {
            continue;
        }
        if cells_with_stockpile.contains(&cell) {
            continue;
        }
        commands.spawn((
            Stockpile {
                kind: AUTO_STOCKPILE_KIND,
                amount: 0,
                capacity: AUTO_STOCKPILE_CAPACITY,
                radius: AUTO_STOCKPILE_RADIUS,
            },
            Transform::from_translation(get_world_from_zone(cell).extend(0.0)),
        ));
    }
}

/// Plugin that wires the hauler systems and the stockpile
/// auto-creation system into the Update schedule. The chain runs
/// after `move_velocity_system` so the movement system has already
/// pruned arrived bots (which is the trigger the arrive and
/// delivery systems wait for).
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
                stockpile_auto_creation_system,
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
}
