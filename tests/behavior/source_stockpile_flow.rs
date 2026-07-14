//! Integration tests for issue #23: Source Stockpiles for Gather work.
//!
//! The Source Stockpile flow ties the gather assignment system to
//! the planned-structure lifecycle from issue #21. The contract:
//!
//!   1. A Worker assigned to a Gather-overlapped deposit checks
//!      for a usable built Source Stockpile before extracting.
//!   2. If no usable built Source Stockpile exists, the demand
//!      system plans a Source Stockpile near the deposit (or
//!      reuses an existing planned one).
//!   3. A Worker claims and builds the Planned Source Stockpile.
//!   4. Once complete, the Source Stockpile becomes a physical
//!      Stockpile owned by the same swarm.
//!   5. The Worker can resume extraction after the Source
//!      Stockpile exists, and the extracted minerals can be
//!      delivered to the completed Source Stockpile.
//!   6. No completed Source Stockpile appears instantly from
//!      Gather paint alone.
//!
//! Each test pins one acceptance bullet so a failure points at a
//! single contract. The tests share a minimal Bevy `App` assembled
//! from `sim_app_with_gather_planned`, which wires Gather +
//! PlannedStructure without the rest of the production or maintenance
//! chain.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, DEFAULT_PLANNED_WORK_TICKS, ExtractProgress, GatherAssignment, Health, Nanobot,
        NanobotType, OwnerSwarm, PlannedKind, PlannedStructure, PlannedStructureClaim,
        PlannedStructureProgress, SOURCE_STOCKPILE_JITTER_AMPLITUDE,
        SOURCE_STOCKPILE_PLACEMENT_RADIUS, Swarm, SwarmId, SwarmMember, VelocityComponent,
        completed_visual_color,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
};

#[path = "../common/mod.rs"]
mod common;

/// Bot speed from the default game settings. Pulled into a
/// constant so the travel-time math in the test is obvious.
const BOT_SPEED: f32 = 5.0;

/// Distance the gather worker has to walk to reach the planned
/// Source Stockpile from the deposit (or back). The demand
/// system places the planned structure on the placement ring
/// at [`SOURCE_STOCKPILE_PLACEMENT_RADIUS`] from the deposit,
/// plus a deterministic jitter of up to
/// [`SOURCE_STOCKPILE_JITTER_AMPLITUDE`]. The travel-time math
/// uses the worst case (ring radius + max jitter) so the
/// worker has arrived by the time the test checks for the
/// completed build, regardless of the specific jitter draw.
const PLANNED_TRAVEL_DISTANCE: f32 =
    SOURCE_STOCKPILE_PLACEMENT_RADIUS + SOURCE_STOCKPILE_JITTER_AMPLITUDE;

/// Ticks of simulation needed for the worker to walk
/// `distance` world units at `BOT_SPEED`. The arrival is
/// "distance / speed" rounded up to the next tick because
/// the movement system only prunes `DirectMovementComponent`
/// on the tick the bot reaches its target.
fn travel_ticks(distance: f32) -> u32 {
    (distance / BOT_SPEED).ceil() as u32 + 1
}

fn build_app() -> App {
    common::sim_app_with_gather_planned()
}

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
}

fn spawn_swarm_and_worker(app: &mut App, worker_pos: Vec2) -> (Entity, Entity) {
    let swarm = common::spawn_swarm_at(app, worker_pos);
    let worker = common::spawn_worker_at(app, worker_pos);
    (swarm, worker)
}

#[test]
fn no_completed_source_stockpile_from_gather_paint_alone() {
    // Acceptance: "no completed Source Stockpile appears
    // instantly from Gather paint alone". Painting Gather
    // intent with no deposit and no worker must not create a
    // Stockpile (completed) or a PlannedStructure of any
    // kind. The demand system only plans for deposits that
    // have a Worker with a GatherAssignment, so without
    // either, nothing emerges.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    assert_eq!(
        world.query::<&Stockpile>().iter(world).count(),
        0,
        "Gather paint alone must not create a completed Stockpile"
    );
    assert_eq!(
        world.query::<&PlannedStructure>().iter(world).count(),
        0,
        "Gather paint alone must not create a PlannedStructure"
    );
}

#[test]
fn opponent_gather_demand_creates_opponent_owned_source_plan() {
    let mut app = build_app();
    let player_swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let opponent_id = SwarmId(1);
    let opponent_swarm = app
        .world_mut()
        .spawn((
            Swarm {},
            opponent_id,
            Transform::from_translation(Vec3::new(512.0, 0.0, 0.0)),
        ))
        .id();
    assert_ne!(player_swarm, opponent_swarm);
    let cell = IVec2::new(0, 0);
    let deposit_pos = common::cell_world_center(cell);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        cell,
        IntentKind::Gather,
        Some(opponent_id),
    );
    let player_worker = common::spawn_worker_at(&mut app, deposit_pos);
    app.world_mut().spawn((
        Nanobot {},
        NanobotType::Worker,
        Commitment::Idle,
        VelocityComponent::default(),
        Health::default(),
        SwarmMember(opponent_id),
        Transform::from_translation(deposit_pos.extend(0.0)),
    ));
    common::spawn_deposit(&mut app, deposit_pos, 100);

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &OwnerSwarm)>();
    let (_planned, owner) = q
        .iter(world)
        .find(|(planned, _)| planned.kind == PlannedKind::SourceStockpile)
        .expect("opponent gather demand must create a Source Stockpile plan");
    assert_eq!(
        owner.0, opponent_swarm,
        "opponent gather demand must stamp opponent OwnerSwarm, not player"
    );
    assert!(
        world
            .entity(player_worker)
            .get::<PlannedStructureClaim>()
            .is_none(),
        "player Worker must not claim opponent Source Stockpile plan"
    );
}

#[test]
fn no_planned_source_stockpile_from_deposit_without_worker() {
    // The demand system is "demand-driven" rather than
    // "intent-driven": a Planned Source Stockpile is only
    // created when a Worker is actually assigned to the
    // deposit, not when a deposit exists. This pins the
    // "Gather intent alone is not enough" half of the
    // contract.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    assert_eq!(
        world.query::<&Stockpile>().iter(world).count(),
        0,
        "deposit without a worker must not create a completed Stockpile"
    );
    assert_eq!(
        world.query::<&PlannedStructure>().iter(world).count(),
        0,
        "deposit without a worker must not create a PlannedStructure"
    );
}

#[test]
fn gather_assignment_triggers_planned_source_stockpile() {
    // Acceptance: "If no usable Source Stockpile exists, a
    // Planned Source Stockpile is created or reused."
    // A Worker assigned to a Gather-overlapped deposit with
    // no usable Source Stockpile must cause a Planned
    // Source Stockpile to appear near the deposit.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (swarm, worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Transform, Option<&OwnerSwarm>)>();
    let mut planned: Option<(IVec2, Vec2, Option<Entity>)> = None;
    for (p, t, owner) in q.iter(world) {
        if p.kind != PlannedKind::SourceStockpile {
            continue;
        }
        planned = Some((p.cell, t.translation.truncate(), owner.map(|o| o.0)));
        break;
    }
    let (planned_cell, planned_pos, planned_owner) = planned
        .expect("Planned Source Stockpile must appear once a Worker is assigned to a deposit");
    assert_eq!(
        planned_cell, cell,
        "Planned Source Stockpile must live in the same cell as the deposit"
    );
    // Issue #24: the placement algorithm picks a position on
    // the ring at `SOURCE_STOCKPILE_PLACEMENT_RADIUS` from the
    // deposit, plus a deterministic jitter of up to
    // `SOURCE_STOCKPILE_JITTER_AMPLITUDE`. The test pins the
    // new "inside the gather zone, at the ring distance"
    // contract rather than the v0 "exact offset" contract.
    let ring_distance = (planned_pos - deposit_pos).length();
    let min_distance = SOURCE_STOCKPILE_PLACEMENT_RADIUS - SOURCE_STOCKPILE_JITTER_AMPLITUDE;
    let max_distance = SOURCE_STOCKPILE_PLACEMENT_RADIUS + SOURCE_STOCKPILE_JITTER_AMPLITUDE;
    assert!(
        ring_distance >= min_distance - 1.0 && ring_distance <= max_distance + 1.0,
        "Planned Source Stockpile must be placed on the placement ring within jitter; \
         got distance={ring_distance} from deposit (expected in [{min_distance}, {max_distance}])"
    );
    assert_eq!(
        planned_owner,
        Some(swarm),
        "Planned Source Stockpile must be owned by the swarm"
    );
    // The worker has a GatherAssignment pointing at the deposit
    // and the deposit exists; the demand system saw the
    // assignment before planning.
    let _ = (worker, deposit);
}

#[test]
fn worker_builds_planned_source_stockpile() {
    // Acceptance: "One Worker claims and builds the Planned
    // Source Stockpile." The same Worker that triggered the
    // demand claims the Planned Source Stockpile, walks to
    // it, spends worker time, and the planned structure
    // promotes to a physical Stockpile. The "one worker"
    // half of the contract is also pinned by the planned
    // structure foundation's reservation tests; here we
    // verify the build completes end-to-end.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Run enough ticks for: assign -> demand -> claim ->
    // travel to planned -> work -> promotion.
    //   assign + demand: 1 tick
    //   travel: travel_ticks(PLANNED_TRAVEL_DISTANCE) ticks
    //   work: DEFAULT_PLANNED_WORK_TICKS ticks
    //   margin: a small buffer
    let total_ticks = 1 + travel_ticks(PLANNED_TRAVEL_DISTANCE) + DEFAULT_PLANNED_WORK_TICKS + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let still_planned = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SourceStockpile)
        .count();
    assert_eq!(
        still_planned, 0,
        "Planned Source Stockpile must be promoted to a Stockpile after the build"
    );
    // Exactly one Source Stockpile (Stockpile entity) exists
    // at the planned position. The completed visual is the
    // signal the build finished.
    let mut q = world.query::<(&Stockpile, &Transform, &Sprite)>();
    let (stockpile, _transform, sprite) = q
        .iter(world)
        .next()
        .expect("a completed Source Stockpile must exist after the build");
    assert_eq!(stockpile.kind, ResourceKind::Minerals);
    assert_eq!(stockpile.amount, 0, "completed Stockpile starts empty");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed visual must be the completed color"
    );
}

#[test]
fn worker_resumes_extraction_after_stockpile_built() {
    // Acceptance: "The Worker can resume extraction after the
    // Source Stockpile exists." After the build finishes,
    // the Worker walks back to the deposit and the gather
    // arrive system inserts `ExtractProgress`. The deposit's
    // amount is the visible end of the resume.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Build phase + travel back to deposit + a buffer of
    // extract ticks so the resume is observable in the
    // deposit's `amount`.
    let total_ticks = 1
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + DEFAULT_PLANNED_WORK_TICKS
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + 10;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let deposit_state = world
        .entity(deposit)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
        .unwrap();
    assert!(
        deposit_state.amount < 100,
        "Worker must resume extraction after the Source Stockpile is built; deposit drained from 100 to {}",
        deposit_state.amount
    );
}

#[test]
fn worker_delivers_minerals_to_completed_source_stockpile() {
    // Acceptance: "Extracted minerals can be delivered to
    // the completed Source Stockpile." The full round trip:
    // build the Source Stockpile, extract, carry, deliver.
    // The Stockpile's `amount` is the visible end of the
    // delivery.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Build + extract + carry + deliver. The test stops right
    // after the first delivery so the ledger assertion sees a
    // single delivery (stockpile = 4, ledger = 4) before a
    // second extract cycle decrements the ledger back toward
    // zero. The "extract + delivery = 4" invariant is the
    // contract: the swarm's minerals move from deposit to
    // stockpile via the worker, and the ledger tracks the
    // delivery end of that move.
    let total_ticks = 1
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + DEFAULT_PLANNED_WORK_TICKS
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)
        + 4  // extract to fill a load
        + 1  // carry-assign tick
        + travel_ticks(PLANNED_TRAVEL_DISTANCE)  // travel to stockpile
        + 1  // delivery
        + 3; // buffer: a few ticks after the delivery, before the next extract
    for _ in 0..total_ticks {
        app.update();
    }

    // Take both queries before the assertions so the
    // immutable borrow on `app.world()` is released.
    let mut q = app.world_mut().query::<&Stockpile>();
    let mut dq = app.world_mut().query::<&ResourceDeposit>();
    let (stockpile_count, stockpile_amount, deposit_remaining) = {
        let world = app.world();
        let count = q.iter(world).count();
        let amount = q.iter(world).next().map(|s| s.amount).unwrap_or(0);
        let deposit: u32 = dq.iter(world).map(|d| d.amount).sum();
        (count, amount, deposit)
    };
    assert_eq!(stockpile_count, 1, "exactly one Source Stockpile exists");
    assert!(
        stockpile_amount > 0,
        "Worker must deliver extracted minerals to the completed Source Stockpile; got amount={}",
        stockpile_amount
    );
    // Resource conservation: every unit extracted from
    // the deposit ends up in the stockpile (or still in
    // flight via WorkerLoad). The deposit + stockpile
    // total must equal the original 100. Issue #38
    // / ADR-0004 changes the worker arrival threshold
    // from `STOP_THRESHOLD` (2) to `deposit.radius` (32),
    // which lets the worker deliver 2 trips in the test
    // timeline (the original timeline assumed a tighter
    // `STOP_THRESHOLD` and stopped right after the first
    // delivery). The conservation invariant is the
    // load-bearing assertion; the exact delivery count
    // is timing-dependent and not the contract.
    assert_eq!(
        deposit_remaining + stockpile_amount,
        100,
        "deposit + stockpile must equal the original 100 (conservation)"
    );
}

#[test]
fn planned_source_stockpile_is_reused_for_nearby_deposits() {
    // Acceptance: "If no usable Source Stockpile exists, a
    // Planned Source Stockpile is created or reused."
    // Two deposits in the same area share a single Planned
    // Source Stockpile; the demand system does not pile
    // multiple plans around the same gather site.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let _deposit_a = common::spawn_deposit(&mut app, deposit_pos, 100);
    let _deposit_b = common::spawn_deposit(&mut app, deposit_pos + Vec2::new(40.0, 0.0), 100);
    // Two workers so both deposits can be assigned at once.
    let _worker_a = common::spawn_worker_at(&mut app, deposit_pos);
    let _worker_b = common::spawn_worker_at(&mut app, deposit_pos + Vec2::new(40.0, 0.0));

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SourceStockpile)
        .count();
    assert_eq!(
        planned_count, 1,
        "two nearby deposits must share a single Planned Source Stockpile, not pile two plans"
    );
}

#[test]
fn source_stockpile_stays_in_gather_painted_cell() {
    // The PRD says "Source Stockpiles should be placed
    // inside the Gather Zone". The placement offset is
    // small enough that the planned structure's world
    // position is inside the same intent grid cell as the
    // deposit, so the "inside the Gather Zone" contract
    // holds for v1 (the Gather Zone is the painted cell).
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    for _ in 0..5 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let (planned, transform) = q
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::SourceStockpile)
        .expect("Planned Source Stockpile must appear in the Gather-painted cell");
    let planned_cell = top_down_2d_rts_prototype_nano_swarm::nanobot::world_to_cell(
        transform.translation.truncate(),
    );
    assert_eq!(
        planned_cell, cell,
        "Planned Source Stockpile must live in the Gather-painted cell"
    );
    assert_eq!(planned.cell, cell);
}

#[test]
fn completed_source_stockpile_keeps_swarm_ownership() {
    // Acceptance: "Once complete, the Source Stockpile
    // becomes a physical Stockpile owned by the same
    // swarm." The promotion path preserves the planned
    // structure's `OwnerSwarm` on the completed
    // `Stockpile`, so the ownership is stable across the
    // build.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Build phase + buffer.
    let total_ticks = 1 + travel_ticks(PLANNED_TRAVEL_DISTANCE) + DEFAULT_PLANNED_WORK_TICKS + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    let owner = world
        .query::<(&Stockpile, &OwnerSwarm)>()
        .iter(world)
        .next()
        .map(|(_, o)| o.0)
        .expect("completed Source Stockpile must keep the swarm's OwnerSwarm");
    assert_eq!(
        owner, swarm,
        "completed Source Stockpile must be owned by the same swarm as the planned structure"
    );
}

#[test]
fn scenario_sized_deposit_at_authored_cell_center_gets_source_stockpile() {
    let mut app = build_app();
    let cell = top_down_2d_rts_prototype_nano_swarm::scenario::PLAYER_DEPOSIT_CELL;
    paint_gather(&mut app, cell);
    let deposit_pos = top_down_2d_rts_prototype_nano_swarm::scenario::cell_origin(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, Vec2::ZERO);
    let deposit = app
        .world_mut()
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 100,
                capacity: 100,
                radius: top_down_2d_rts_prototype_nano_swarm::scenario::STARTING_WORK_RADIUS,
            },
            Transform::from_translation(deposit_pos.extend(0.0)),
        ))
        .id();

    app.update();

    let world = app.world_mut();
    let mut q = world.query::<(&PlannedStructure, &Transform)>();
    let plan_pos = q
        .iter(world)
        .find_map(|(planned, transform)| {
            (planned.kind == PlannedKind::SourceStockpile)
                .then_some(transform.translation.truncate())
        })
        .expect("scenario-authored deposit must still get a Planned Source Stockpile");
    let required_gap = top_down_2d_rts_prototype_nano_swarm::scenario::STARTING_WORK_RADIUS
        + top_down_2d_rts_prototype_nano_swarm::nanobot::SOURCE_STOCKPILE_FOOTPRINT_RADIUS
        + top_down_2d_rts_prototype_nano_swarm::nanobot::SOURCE_STOCKPILE_PADDING;
    assert!(
        plan_pos.distance(deposit_pos) >= required_gap,
        "planned Source Stockpile must not overlap scenario-sized deposit; plan={plan_pos:?} deposit={deposit_pos:?} required_gap={required_gap}"
    );
    drop(q);

    for _ in 0..500 {
        app.update();
    }

    let world = app.world_mut();
    assert!(
        world
            .entity(deposit)
            .get::<ResourceDeposit>()
            .unwrap()
            .amount
            < 100,
        "deposit amount should decrease once extraction resumes instead of worker idling at deposit"
    );
}

#[test]
fn worker_waits_for_planned_source_stockpile_before_extracting() {
    // While the Planned Source Stockpile is being built,
    // the Worker must not start extracting: the deposit
    // amount stays at its initial value, and the Worker
    // has no `ExtractProgress` component.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Run a small number of ticks -- enough for the
    // assignment + demand + claim to fire, but well short
    // of the build finishing.
    let pre_build_ticks = 1 + travel_ticks(PLANNED_TRAVEL_DISTANCE) / 2;
    for _ in 0..pre_build_ticks {
        app.update();
    }

    let world = app.world_mut();
    let deposit_state = world
        .entity(deposit)
        .get::<top_down_2d_rts_prototype_nano_swarm::resources::ResourceDeposit>()
        .unwrap();
    assert_eq!(
        deposit_state.amount, 100,
        "Worker must not extract from the deposit while the Source Stockpile is still being built"
    );
    let extract = world.entity(worker).get::<ExtractProgress>();
    assert!(
        extract.is_none(),
        "Worker must not be in ExtractProgress while the Source Stockpile is still being built"
    );
    // The Worker either still has a GatherAssignment (if the
    // planned structure is being built by another worker) or
    // has transitioned to PlannedStructureClaim/Progress
    // (if the Worker itself is the builder). Both are valid
    // "not extracting" states.
    let gather = world.entity(worker).get::<GatherAssignment>();
    let claim = world.entity(worker).get::<PlannedStructureClaim>();
    let progress = world.entity(worker).get::<PlannedStructureProgress>();
    let is_busy = gather.is_some() || claim.is_some() || progress.is_some();
    assert!(
        is_busy,
        "Worker must be assigned or building the planned structure while waiting"
    );
    // The planned structure is still in the world (not yet
    // promoted) during the build phase.
    let planned = world
        .query::<&PlannedStructure>()
        .iter(world)
        .find(|p| p.kind == PlannedKind::SourceStockpile);
    assert!(
        planned.is_some(),
        "Planned Source Stockpile must still exist during the build phase"
    );
}

#[test]
fn second_worker_can_claim_and_build_planned_source_stockpile() {
    // The "one Worker" claim contract is the foundation
    // slice's reservation tests; this test pins the
    // cross-component hand-off: the first Worker triggers
    // the demand, the second Worker (the only idle Worker
    // once the first is assigned) claims the planned
    // structure and builds it. The first Worker waits at
    // the deposit and resumes extraction once the build
    // finishes.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker_assigned) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    // A second worker far from the deposit so the planned
    // claim system picks it (the first worker is busy with
    // a GatherAssignment, so it is filtered out of the
    // claim query).
    let far_pos = deposit_pos + Vec2::new(400.0, 0.0);
    let second_worker = common::spawn_worker_at(&mut app, far_pos);

    // Build phase + buffer.
    let total_ticks = 1 + travel_ticks(PLANNED_TRAVEL_DISTANCE) + DEFAULT_PLANNED_WORK_TICKS + 5;
    for _ in 0..total_ticks {
        app.update();
    }

    let world = app.world_mut();
    // The planned structure is promoted.
    let still_planned = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SourceStockpile)
        .count();
    assert_eq!(
        still_planned, 0,
        "Planned Source Stockpile must be promoted to a Stockpile after the second worker builds it"
    );
    // The second worker is no longer building.
    let _ = second_worker;
    let progress = world
        .entity(second_worker)
        .get::<PlannedStructureProgress>();
    let claim = world.entity(second_worker).get::<PlannedStructureClaim>();
    assert!(
        progress.is_none() && claim.is_none(),
        "second worker must be released after the build"
    );
}

#[test]
fn demand_system_does_not_double_plan_when_planned_already_exists() {
    // The "reused" half of the contract: a second deposit
    // whose planned structure is already planned (via a
    // first deposit in the same area) must not cause a
    // second Planned Source Stockpile. The demand system
    // checks for any planned Source Stockpile within
    // proximity and skips planning if one exists.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let _deposit_a = common::spawn_deposit(&mut app, deposit_pos, 100);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);

    // First tick of demand.
    for _ in 0..3 {
        app.update();
    }
    let planned_after_first = {
        let world = app.world_mut();
        world
            .query::<&PlannedStructure>()
            .iter(world)
            .filter(|p| p.kind == PlannedKind::SourceStockpile)
            .count()
    };
    assert_eq!(
        planned_after_first, 1,
        "first deposit must create exactly one Planned Source Stockpile"
    );

    // Add a second deposit in the same area, give it its
    // own worker, and assert the demand system does not
    // plan a second Source Stockpile.
    let deposit_b_pos = deposit_pos + Vec2::new(40.0, 0.0);
    let _deposit_b = common::spawn_deposit(&mut app, deposit_b_pos, 100);
    let _worker_b = common::spawn_worker_at(&mut app, deposit_b_pos);

    for _ in 0..5 {
        app.update();
    }
    let planned_after_second = {
        let world = app.world_mut();
        world
            .query::<&PlannedStructure>()
            .iter(world)
            .filter(|p| p.kind == PlannedKind::SourceStockpile)
            .count()
    };
    assert_eq!(
        planned_after_second, 1,
        "second nearby deposit must reuse the existing Planned Source Stockpile, not plan another"
    );
}
