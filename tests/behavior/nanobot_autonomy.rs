//! Integration tests for issue #6: Nanobot Types and dumb autonomy scoring.
//!
//! These tests drive the new [`NanobotType`], [`Commitment`], and
//! [`SoftWorkSlots`] types through a minimal Bevy `App` (no rendering
//! plugins, no zone material) and assert the global scoring contract:
//! idle nanobots respond immediately, carrying/working nanobots are
//! biased toward finishing, and soft work slot pressure makes
//! overcrowded work less attractive without ever hard-rejecting it.
//!
//! The test setup deliberately avoids spawning nanobots as full
//! `NanobotBundle`s. The scoring contract is a pure function of the
//! inputs we care about (type, commitment, position, slot count), so
//! we drive it directly through `best_candidate` and the `SoftWorkSlots`
//! resource. Adding full bundle spawning would test Bevy entity
//! bookkeeping, not the autonomy contract.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind, PAINT_STRENGTH_CAP},
    nanobot::{best_candidate, Commitment, NanobotType, SoftWorkSlots},
};

#[path = "../common/mod.rs"]
mod common;

const CELL_SIZE: f32 = 1.0;

fn build_app() -> App {
    common::minimal_app()
}

#[test]
fn nanobot_type_component_inserts_and_queries() {
    // The glossary is explicit: every nanobot has exactly one of
    // Worker, Hauler, Defender. A Bevy entity holding a `NanobotType`
    // must round-trip through a real world and be queryable.
    let mut world = World::new();
    let e_worker = world
        .spawn((NanobotType::Worker, Commitment::Idle, Transform::default()))
        .id();
    let e_hauler = world
        .spawn((NanobotType::Hauler, Commitment::Idle, Transform::default()))
        .id();
    let e_defender = world
        .spawn((
            NanobotType::Defender,
            Commitment::Idle,
            Transform::default(),
        ))
        .id();

    let mut q = world.query::<&NanobotType>();
    assert_eq!(*q.get(&world, e_worker).unwrap(), NanobotType::Worker);
    assert_eq!(*q.get(&world, e_hauler).unwrap(), NanobotType::Hauler);
    assert_eq!(*q.get(&world, e_defender).unwrap(), NanobotType::Defender);
}

#[test]
fn soft_work_slots_resource_round_trips_through_app() {
    let mut app = build_app();
    let cell = IVec2::new(0, 0);

    // Resource starts empty.
    {
        let slots = app.world().resource::<SoftWorkSlots>();
        assert!(slots.is_empty());
        assert_eq!(slots.occupied(cell, IntentKind::Gather), 0);
    }

    // Occupy through the resource handle, not a local copy.
    {
        let mut slots = app.world_mut().resource_mut::<SoftWorkSlots>();
        slots.occupy(cell, IntentKind::Gather);
        slots.occupy(cell, IntentKind::Gather);
    }
    app.update();

    let slots = app.world().resource::<SoftWorkSlots>();
    assert_eq!(slots.occupied(cell, IntentKind::Gather), 2);
    assert_eq!(slots.len(), 1);

    // Release through the resource as well.
    {
        let mut slots = app.world_mut().resource_mut::<SoftWorkSlots>();
        slots.release(cell, IntentKind::Gather);
    }
    app.update();
    let slots = app.world().resource::<SoftWorkSlots>();
    assert_eq!(slots.occupied(cell, IntentKind::Gather), 1);
}

#[test]
fn idle_worker_responds_immediately_to_nearby_gather_paint() {
    // Acceptance: "Idle nanobots can immediately respond to useful
    // global intent". A single idle worker near a strongly painted
    // Gather cell must pick that cell as its candidate.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    let picked = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        Vec2::new(0.0, 0.0),
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("idle worker must find a candidate");
    assert_eq!(picked.cell, cell);
    assert_eq!(picked.kind, IntentKind::Gather);
    assert!(picked.score > 0.0);
}

#[test]
fn idle_worker_picks_closest_cell_when_two_have_equal_paint() {
    // "Nearby or idle nanobots usually respond first" -- of two
    // equally painted cells, the closer one wins.
    let mut app = build_app();
    let near = IVec2::new(0, 0);
    let far = IVec2::new(2, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(near, IntentKind::Gather, 8);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(far, IntentKind::Gather, 8);

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    let picked = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        Vec2::new(0.0, 0.0),
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("must find a candidate");
    assert_eq!(picked.cell, near);
}

#[test]
fn idle_worker_picks_stronger_paint_when_two_are_equidistant() {
    // Two cells at the same distance: the more strongly painted one
    // wins because paint strength is a positive linear factor.
    let mut app = build_app();
    let weak = IVec2::new(-1, 0);
    let strong = IVec2::new(1, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(weak, IntentKind::Gather, 2);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        strong,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    let pos = Vec2::new(0.0, 0.0);
    let picked = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("must find a candidate");
    assert_eq!(picked.cell, strong);
    assert!(picked.paint_strength > 0);
}

#[test]
fn soft_work_slot_pressure_reduces_score_but_never_rejects() {
    // Acceptance: "Soft work slots reduce usefulness of overcrowded
    // work without hard invisible rejection". The crowded cell must
    // stay pickable (no None), but it must score strictly less than
    // an empty cell with the same paint.
    let mut app = build_app();
    let a = IVec2::new(-1, 0);
    let b = IVec2::new(1, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(a, IntentKind::Gather, 8);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(b, IntentKind::Gather, 8);

    // Pile 5 nanobots on cell `a` for Gather.
    {
        let mut slots = app.world_mut().resource_mut::<SoftWorkSlots>();
        for _ in 0..5 {
            slots.occupy(a, IntentKind::Gather);
        }
    }
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    let pos = Vec2::new(0.0, 0.0);
    let picked = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("crowded cell must still be pickable -- no hard rejection");
    // The empty cell must beat the crowded one.
    assert_eq!(picked.cell, b);
    assert!(picked.score > 0.0);
}

#[test]
fn type_fit_routes_defender_to_defend_and_worker_to_gather() {
    // Acceptance: "Nanobots have Worker, Hauler, or Defender type".
    // Each type must pick the layer it is fit for, even when the
    // other layers are equally painted nearby.
    let mut app = build_app();
    let gather = IVec2::new(-1, 0);
    let defend = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        gather,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        defend,
        IntentKind::Defend,
        PAINT_STRENGTH_CAP,
    );

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    let pos = Vec2::new(0.0, 0.0);

    let worker_pick = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("worker must find a candidate");
    assert_eq!(worker_pick.cell, gather);
    assert_eq!(worker_pick.kind, IntentKind::Gather);

    let defender_pick = best_candidate(
        grid,
        NanobotType::Defender,
        Commitment::Idle,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("defender must find a candidate");
    assert_eq!(defender_pick.cell, defend);
    assert_eq!(defender_pick.kind, IntentKind::Defend);
}

#[test]
fn commitment_ordering_idle_above_working_above_carrying() {
    // Acceptance: "Carrying nanobots usually finish delivery before
    // reassessing" and "Active workers finish short work chunks
    // before reassessing". Both contracts collapse into one ordering
    // test: idle > working > carrying for the same painted cell.
    let mut app = build_app();
    let cell = IVec2::new(0, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        PAINT_STRENGTH_CAP,
    );
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();
    let pos = Vec2::new(0.0, 0.0);

    let idle = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("idle must pick");
    let working = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Working,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("working must pick");
    let carrying = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Carrying,
        pos,
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("carrying must pick");

    assert!(idle.score > working.score);
    assert!(working.score > carrying.score);
}

#[test]
fn mixed_swarm_picks_through_type_fit_and_distance_together() {
    // End-to-end smoke: simulate one bot per type at distinct
    // positions and verify each picks the cell+kind it is fit for
    // closest to it. This is the contract: nanobots have types,
    // know about painted intent globally, and prefer the best
    // (type-fit, distance, paint) combination.
    let mut app = build_app();
    // Worker territory: Gather paint near the origin.
    let worker_gather = IVec2::new(0, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(worker_gather, IntentKind::Gather, 12);
    // Hauler territory: Corridor paint slightly to the right.
    let hauler_corridor = IVec2::new(2, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(hauler_corridor, IntentKind::Corridor, 12);
    // Defender territory: Defend paint further to the right.
    // The 6x6 grid spans [-3, 3), so cell (2, 0) is the farthest
    // x-axis cell we can use for a clearly distinct defender.
    let defender_defend = IVec2::new(2, 2);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(defender_defend, IntentKind::Defend, 12);
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let slots = app.world().resource::<SoftWorkSlots>();

    let worker_pick = best_candidate(
        grid,
        NanobotType::Worker,
        Commitment::Idle,
        Vec2::new(0.0, 0.0),
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("worker must pick");
    assert_eq!(worker_pick.cell, worker_gather);
    assert_eq!(worker_pick.kind, IntentKind::Gather);

    let hauler_pick = best_candidate(
        grid,
        NanobotType::Hauler,
        Commitment::Idle,
        Vec2::new(2.0, 0.0),
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("hauler must pick");
    assert_eq!(hauler_pick.cell, hauler_corridor);
    assert_eq!(hauler_pick.kind, IntentKind::Corridor);

    let defender_pick = best_candidate(
        grid,
        NanobotType::Defender,
        Commitment::Idle,
        Vec2::new(2.0, 2.0),
        slots,
        CELL_SIZE,
        &IntentKind::ALL,
    )
    .expect("defender must pick");
    assert_eq!(defender_pick.cell, defender_defend);
    assert_eq!(defender_pick.kind, IntentKind::Defend);
}
