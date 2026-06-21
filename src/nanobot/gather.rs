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

use crate::intent::{IntentGrid, IntentKind};
use crate::nanobot::autonomy::{best_candidate, Commitment, NanobotType, SoftWorkSlots};
use crate::nanobot::components::{DirectMovementComponent, Nanobot, SwarmMember};
use crate::resources::{ResourceDeposit, ResourceKind, ResourceLedger, Stockpile};
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
        ),
    >,
    deposits: Query<(&ResourceDeposit, &Transform)>,
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

        let distance = transform
            .translation
            .truncate()
            .distance(deposit_transform.translation.truncate());
        if distance <= deposit.radius {
            commands.entity(entity).insert(ExtractProgress::default());
        }
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
pub struct GatherPlugin;

impl Plugin for GatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                worker_gather_assignment_system,
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
