//! Integration tests for issue #24: Source Stockpile placement rules.
//!
//! The placement rules are:
//!   1. Candidate placements are generated around the deposit
//!      at a configured Source Stockpile distance.
//!   2. Placement uses deterministic jitter that is stable
//!      across ticks.
//!   3. Candidate centers must be inside the owning swarm's
//!      Gather Zone.
//!   4. Candidates overlapping existing structures are
//!      rejected.
//!   5. Candidates overlapping Planned Structures are rejected.
//!   6. Candidate rejection includes padding between footprints.
//!   7. Candidate scoring prefers the haul/base/Build Zone
//!      direction when available.
//!   8. If no valid candidate exists, no Planned Source
//!      Stockpile is created.
//!
//! The pure-helper contracts (ring generation, jitter stability,
//! zone containment, overlap rejection, padding, haul-direction
//! bias) are covered by the unit tests in
//! `src/nanobot/placement.rs`. The integration tests below
//! verify the demand system runs the algorithm correctly and
//! observes the "no overlapping fallback" half of the contract
//! when the algorithm returns None.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{
        completed_visual_color, GatherAssignment, PlannedKind, PlannedStructure, SwarmId,
        SOURCE_STOCKPILE_JITTER_AMPLITUDE, SOURCE_STOCKPILE_PADDING,
        SOURCE_STOCKPILE_PLACEMENT_RADIUS,
    },
    resources::Stockpile,
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    common::sim_app_with_gather_planned()
}

fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
        Some(SwarmId::PLAYER),
    ));
}

fn paint_build(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(
        cell,
        IntentKind::Build,
        PAINT_STRENGTH_CAP,
        Some(SwarmId::PLAYER),
    ));
}

fn spawn_swarm_and_worker(app: &mut App, worker_pos: Vec2) -> (Entity, Entity) {
    let swarm = common::spawn_swarm_at(app, worker_pos);
    let worker = common::spawn_worker_at(app, worker_pos);
    (swarm, worker)
}

/// World position of the planned Source Stockpile, if one
/// exists. Returns `None` when the demand system did not plan
/// (which is the "no valid candidate" half of the contract).
fn planned_source_stockpile_position(app: &mut App) -> Option<Vec2> {
    let world = app.world_mut();
    world
        .query::<(&PlannedStructure, &Transform)>()
        .iter(world)
        .find(|(p, _)| p.kind == PlannedKind::SourceStockpile)
        .map(|(_, t)| t.translation.truncate())
}

#[test]
fn candidate_placements_lie_on_the_configured_ring() {
    // Acceptance: "Candidate placements are generated around
    // the deposit at the configured Source Stockpile
    // distance." Every viable candidate (and therefore the
    // chosen one) sits on the placement ring at
    // [`SOURCE_STOCKPILE_PLACEMENT_RADIUS`] from the
    // deposit, plus a deterministic jitter of up to
    // [`SOURCE_STOCKPILE_JITTER_AMPLITUDE`].
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    for _ in 0..5 {
        app.update();
    }

    let pos = planned_source_stockpile_position(&mut app)
        .expect("Planned Source Stockpile must be created for a gather-overlapped deposit");
    let distance = (pos - deposit_pos).length();
    let min_d = SOURCE_STOCKPILE_PLACEMENT_RADIUS - SOURCE_STOCKPILE_JITTER_AMPLITUDE;
    let max_d = SOURCE_STOCKPILE_PLACEMENT_RADIUS + SOURCE_STOCKPILE_JITTER_AMPLITUDE;
    assert!(
        (min_d..=max_d).contains(&distance),
        "planned position must be on the placement ring within jitter; \
         got distance={distance}, expected in [{min_d}, {max_d}]"
    );
}

#[test]
fn placement_jitter_is_stable_across_ticks() {
    // Acceptance: "Placement uses deterministic jitter that
    // is stable across ticks." Two demand runs with the same
    // swarm state produce the same planned position. We
    // model the "later tick" by removing the planned
    // structure (without changing the swarm) and re-running
    // the demand system; the new plan lands at the same
    // position.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    for _ in 0..3 {
        app.update();
    }
    let pos_first = planned_source_stockpile_position(&mut app)
        .expect("first planning pass must produce a Planned Source Stockpile");

    // Remove the planned structure and re-run. The new
    // pass sees the same deposit, gather cell, and obstacle
    // list, so the chosen position must match.
    {
        let world = app.world_mut();
        let mut q = world.query::<(Entity, &PlannedStructure)>();
        let to_remove: Vec<Entity> = q
            .iter(world)
            .filter(|(_, p)| p.kind == PlannedKind::SourceStockpile)
            .map(|(e, _)| e)
            .collect();
        for e in to_remove {
            world.despawn(e);
        }
    }

    for _ in 0..3 {
        app.update();
    }
    let pos_second = planned_source_stockpile_position(&mut app)
        .expect("second planning pass must produce a Planned Source Stockpile");

    assert_eq!(
        pos_first, pos_second,
        "placement must be a pure function of the swarm state, not the tick counter"
    );
}

#[test]
fn candidates_outside_the_gather_zone_are_rejected() {
    // Acceptance: "Candidate centers must be inside the
    // owning swarm's Gather Zone." The deposit lives in a
    // Gather-painted cell, but the swarm's only Gather
    // paint is on a different cell. The ring candidates
    // around the deposit land in the deposit's cell, which
    // is not in the gather list, so every candidate is
    // rejected and no planned structure emerges.
    //
    // The test inserts a [`GatherAssignment`] by hand so the
    // demand system actually processes the deposit; otherwise
    // the assignment system would route the worker to the
    // (empty) gather cell and the demand system would never
    // see the deposit.
    let mut app = build_app();
    let deposit_cell = IVec2::new(0, 0);
    // Use an in-bounds gather cell at (-2, 0). The ring
    // candidates around the deposit stay in the deposit's
    // cell (0, 0), which is not in the gather list.
    let gather_cell = IVec2::new(-2, 0);
    paint_gather(&mut app, gather_cell);
    let deposit_pos = common::cell_world_center(deposit_cell);
    let (_swarm, worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(gather_cell, deposit));

    for _ in 0..5 {
        app.update();
    }

    let planned_count = {
        let world = app.world_mut();
        world
            .query::<&PlannedStructure>()
            .iter(world)
            .filter(|p| p.kind == PlannedKind::SourceStockpile)
            .count()
    };
    assert_eq!(
        planned_count, 0,
        "no Planned Source Stockpile must be created when no candidate is in the gather zone"
    );
}

#[test]
fn haul_direction_bias_picks_aligned_candidate() {
    // Acceptance: "Candidate scoring prefers the
    // haul/base/Build Zone direction when available." A
    // Build-painted cell east of the deposit makes "east"
    // the preferred haul direction, and the algorithm picks
    // the angle-0 (east) candidate.
    //
    // This test uses `sim_app_with_gather` (without the
    // PlannedStructurePlugin) so painting a Build cell
    // does not also auto-spawn a Planned Structure at the
    // Build cell's center -- the demand system is the only
    // source of the planned structure under test, so the
    // chosen position unambiguously reflects the haul-
    // direction bias.
    let mut app = common::sim_app_with_gather();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    // The Build cell is two cells east of the deposit so
    // the haul direction is unambiguously east.
    let build_cell = IVec2::new(2, 0);
    paint_build(&mut app, build_cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    for _ in 0..5 {
        app.update();
    }

    let pos = planned_source_stockpile_position(&mut app)
        .expect("Planned Source Stockpile must be created");
    let offset = pos - deposit_pos;
    // The east candidate's x offset is the ring radius;
    // the algorithm must pick a position with a positive
    // (eastward) x offset, and its magnitude must be near
    // the ring radius (within jitter).
    assert!(
        offset.x > 0.0,
        "haul direction east must pick an eastward candidate; got offset={offset:?}"
    );
    let expected_mag = SOURCE_STOCKPILE_PLACEMENT_RADIUS;
    let actual_mag = offset.length();
    assert!(
        (actual_mag - expected_mag).abs() <= SOURCE_STOCKPILE_JITTER_AMPLITUDE + 1.0,
        "chosen position must be on the ring within jitter; got magnitude={actual_mag}, \
         expected near {expected_mag}"
    );
}

#[test]
fn no_planned_source_stockpile_when_gather_cell_is_wrong() {
    // Acceptance: "If no valid candidate exists, no
    // Planned Source Stockpile is created." The demand
    // system calls the placement algorithm with the swarm's
    // Gather cells; when the deposit is not in any gather
    // cell, the algorithm rejects every candidate and
    // returns None. The demand system must not spawn a
    // planned structure as a fallback.
    //
    // This test uses `sim_app_with_gather` to avoid the
    // auto-creation of planned structures in Build cells,
    // so the only planned structure (if any) is the one
    // the demand system tries to plan.
    //
    // The test inserts a [`GatherAssignment`] by hand so the
    // demand system actually processes the deposit; otherwise
    // the assignment system would route the worker to the
    // (empty) gather cell and the demand system would never
    // see the deposit.
    let mut app = common::sim_app_with_gather();
    let deposit_cell = IVec2::new(0, 0);
    let gather_cell = IVec2::new(-2, 0);
    paint_gather(&mut app, gather_cell);
    let deposit_pos = common::cell_world_center(deposit_cell);
    let (_swarm, worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(gather_cell, deposit));

    for _ in 0..5 {
        app.update();
    }

    let planned_count = {
        let world = app.world_mut();
        world
            .query::<(&PlannedStructure, &Transform)>()
            .iter(world)
            .filter(|(p, _)| p.kind == PlannedKind::SourceStockpile)
            .count()
    };
    assert_eq!(
        planned_count, 0,
        "no Planned Source Stockpile must be created when placement returns None"
    );
}

#[test]
fn placement_replaces_planned_structure_with_completed_stockpile() {
    // The end-to-end happy path with the new placement:
    // a Planned Source Stockpile is created, a Worker
    // builds it, and the visual flips to the completed
    // color. This pins the "placement still works in the
    // full plan-build-gather-deliver flow" half of the
    // contract.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Drive long enough for the build to complete and
    // the worker to start gathering. We do not pin a
    // specific number of ticks; we just check the
    // post-build state.
    for _ in 0..100 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<(&Stockpile, &Sprite)>();
    let (_stockpile, sprite) = q
        .iter(world)
        .next()
        .expect("a completed Source Stockpile must exist after the build");
    assert_eq!(
        sprite.color,
        completed_visual_color(),
        "completed visual must be the completed color"
    );
}

#[test]
fn no_floating_planned_source_stockpile_after_demand_satisfied() {
    // Once the build finishes, the demand system must
    // not pile a second Planned Source Stockpile on top
    // of the completed one. The "has any near Source
    // Stockpile" check sees the completed stockpile and
    // the demand system stays quiet.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    paint_gather(&mut app, cell);
    let deposit_pos = common::cell_world_center(cell);
    let (_swarm, _worker) = spawn_swarm_and_worker(&mut app, deposit_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);

    // Drive long enough for the build to finish.
    for _ in 0..100 {
        app.update();
    }

    let world = app.world_mut();
    let planned_count = world
        .query::<&PlannedStructure>()
        .iter(world)
        .filter(|p| p.kind == PlannedKind::SourceStockpile)
        .count();
    assert_eq!(
        planned_count, 0,
        "no Planned Source Stockpile must remain after the build completes"
    );
    let stockpile_count = world.query::<&Stockpile>().iter(world).count();
    assert_eq!(
        stockpile_count, 1,
        "exactly one completed Source Stockpile must exist after the build"
    );
}

#[test]
fn placement_constants_form_a_tight_enough_ring_to_stay_in_cell() {
    // The placement constants must keep every ring + jitter
    // candidate inside the deposit's intent grid cell, so
    // the "inside the Gather Zone" filter is a no-op for
    // the v1 single-cell Gather scenario. This pins the
    // invariant so a future tuning pass that changes the
    // constants cannot silently break the cell-containment
    // contract.
    let ring_radius = SOURCE_STOCKPILE_PLACEMENT_RADIUS;
    let jitter = SOURCE_STOCKPILE_JITTER_AMPLITUDE;
    let cell_size = top_down_2d_rts_prototype_nano_swarm::ZONE_BLOCK_SIZE;
    // The deposit is at the cell center; the worst-case
    // jittered candidate is `ring_radius + jitter` away.
    // It must stay inside the half-cell width.
    let worst_case = ring_radius + jitter;
    assert!(
        worst_case < cell_size / 2.0,
        "worst-case candidate (ring {ring_radius} + jitter {jitter} = {worst_case}) \
         must stay inside the half-cell width {}",
        cell_size / 2.0
    );
}

#[test]
fn placement_padding_is_positive() {
    // Pin the tuning assumption: padding must be a
    // positive value, otherwise the rejection threshold
    // degenerates to the no-padding case. A future tuning
    // pass that sets it to 0 would silently break the
    // "padding between footprints" half of the contract.
    const { assert!(SOURCE_STOCKPILE_PADDING > 0.0) };
    const { assert!(SOURCE_STOCKPILE_PADDING < SOURCE_STOCKPILE_PLACEMENT_RADIUS) };
}
