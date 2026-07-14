//! Behavior tests for issue #39: idle nanobots spread out over
//! their zone (cosmetic liveness).
//!
//! The pure decision helpers (region membership, gradient target,
//! stranded seek, tie-break) are pinned by unit tests in
//! `src/nanobot/spread.rs`. These integration tests cover the ECS
//! wiring contracts the unit tests cannot reach:
//!
//! - Only `Commitment::Idle` nanobots without a
//!   `DirectMovementComponent` receive a spread nudge; carrying,
//!   working, and moving bots are never nudged.
//! - A spread nudge never inserts a `DirectMovementComponent` or
//!   changes `Commitment` -- an idle bot stays fully grabbable by
//!   the demand allocator.
//! - A Worker spreads across the merged Gather+Build region; a
//!   Defender ignores Gather paint (Defend-only); the region
//!   boundary follows `fit_for == 1.0`.
//! - A stranded idle bot drifts toward the nearest type-fit cell.
//!
//! The isolated-nudge tests register `idle_spread_system` alone on a
//! [`common::minimal_app`] so the observed `VelocityComponent` is
//! exactly the spread nudge (the full chain's `velocity_system`
//! would otherwise zero it the same frame). The drift test uses the
//! full [`common::sim_app`] chain so the nudge integrates into the
//! bot's `Transform` over many ticks, proving the spread force is
//! consumed the same frame it is produced.

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{Commitment, DirectMovementComponent, VelocityComponent, idle_spread_system},
};

#[path = "../common/mod.rs"]
mod common;

/// World center of `cell`, matching `ai::get_world_from_zone`.
fn center(cell: IVec2) -> Vec2 {
    common::cell_world_center(cell)
}

/// Paint `kind` at `cell`.
fn paint(app: &mut App, cell: IVec2, kind: IntentKind) {
    app.world_mut().resource_mut::<IntentGrid>().add(cell, kind);
}

/// Build a minimal app with only `idle_spread_system` registered, so
/// the test can read the exact nudge written to `VelocityComponent`
/// without `velocity_system` zeroing it the same frame.
fn spread_only_app() -> App {
    let mut app = common::minimal_app();
    app.add_systems(Update, idle_spread_system);
    app
}

#[test]
fn idle_worker_in_fit_region_is_nudged_toward_empty_fit_neighbour() {
    // A lone idle Worker sitting on a Gather cell with a single
    // painted Gather neighbour must be nudged toward that neighbour.
    // Lone bot -> flat density field -> tie-break, and with exactly
    // one fit neighbour the pick is deterministic.
    let mut app = spread_only_app();
    paint(&mut app, IVec2::new(0, 0), IntentKind::Gather);
    paint(&mut app, IVec2::new(1, 0), IntentKind::Gather);
    let bot = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));

    app.update();

    let velocity = app
        .world()
        .entity(bot)
        .get::<VelocityComponent>()
        .expect("worker must have a VelocityComponent")
        .value;
    assert!(
        velocity.x > 0.0,
        "lone worker should nudge east toward the empty fit neighbour; got {:?}",
        velocity
    );
    assert!(
        velocity.y.abs() < 1e-4,
        "nudge toward the same-row east neighbour must be purely horizontal; got {:?}",
        velocity
    );
}

#[test]
fn worker_spreads_across_merged_gather_and_build_region() {
    // The merged worker region: a Worker standing on a Gather cell
    // treats an adjacent Build-only cell as a fit neighbour and
    // nudges onto it. This is the acceptance criterion "a Worker
    // spreads across merged Gather+Build cells".
    let mut app = spread_only_app();
    paint(&mut app, IVec2::new(0, 0), IntentKind::Gather);
    // East neighbour carries Build only -- no Gather. A Worker still
    // fits it (fit_for Build == 1.0).
    paint(&mut app, IVec2::new(1, 0), IntentKind::Build);
    let bot = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));

    app.update();

    let velocity = app
        .world()
        .entity(bot)
        .get::<VelocityComponent>()
        .unwrap()
        .value;
    assert!(
        velocity.x > 0.0,
        "worker should nudge onto the Build neighbour (merged region); got {:?}",
        velocity
    );
}

#[test]
fn hauler_spreads_only_over_corridor() {
    // A Hauler fits Corridor only (the 0.5 Build partial-fit is
    // excluded by the == 1.0 spread rule). Painted Corridor east of
    // the bot nudges it east; painted Gather alone would not.
    let mut app = spread_only_app();
    paint(&mut app, IVec2::new(1, 0), IntentKind::Corridor);
    let bot = common::spawn_hauler_at(&mut app, center(IVec2::new(0, 0)));

    app.update();

    let velocity = app
        .world()
        .entity(bot)
        .get::<VelocityComponent>()
        .unwrap()
        .value;
    // The hauler starts on an unpainted cell, so it is stranded and
    // drifts toward the nearest (only) fit Corridor cell to the east.
    assert!(
        velocity.x > 0.0,
        "hauler should drift toward the Corridor cell; got {:?}",
        velocity
    );
}

#[test]
fn defender_ignores_gather_paint() {
    // A Defender fits Defend only. Surrounded by Gather paint it has
    // no type-fit region, so it receives no nudge. This pins the
    // "Defender over Defend only" boundary from the acceptance
    // criteria.
    let mut app = spread_only_app();
    // Saturate the bot's cell and all 8 neighbours with Gather.
    for dx in -1..=1 {
        for dy in -1..=1 {
            paint(&mut app, IVec2::new(dx, dy), IntentKind::Gather);
        }
    }
    let bot = common::spawn_defender_at(&mut app, center(IVec2::new(0, 0)));

    app.update();

    let velocity = app
        .world()
        .entity(bot)
        .get::<VelocityComponent>()
        .unwrap()
        .value;
    assert!(
        velocity == Vec2::ZERO,
        "defender surrounded by Gather paint must not be nudged; got {:?}",
        velocity
    );
}

#[test]
fn spread_skips_bot_with_direct_movement_component() {
    // Moving bots are never nudged -- the spread query excludes
    // `DirectMovementComponent`. A bot mid-move keeps responding only
    // to the movement system.
    let mut app = spread_only_app();
    paint(&mut app, IVec2::new(0, 0), IntentKind::Gather);
    paint(&mut app, IVec2::new(1, 0), IntentKind::Gather);
    let bot = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));
    app.world_mut()
        .entity_mut(bot)
        .insert(DirectMovementComponent {
            xy: Vec2::new(1000.0, 0.0),
            stop_radius: 0.0,
        });

    app.update();

    let velocity = app
        .world()
        .entity(bot)
        .get::<VelocityComponent>()
        .unwrap()
        .value;
    assert!(
        velocity == Vec2::ZERO,
        "moving bot must not receive a spread nudge; got {:?}",
        velocity
    );
}

#[test]
fn spread_skips_carrying_and_working_bots() {
    // Only `Commitment::Idle` bots spread. Carrying and Working bots
    // are left untouched even when standing on type-fit paint.
    let mut app = spread_only_app();
    paint(&mut app, IVec2::new(0, 0), IntentKind::Gather);
    paint(&mut app, IVec2::new(1, 0), IntentKind::Gather);

    let carrying = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));
    app.world_mut()
        .entity_mut(carrying)
        .insert(Commitment::Carrying);

    let working = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));
    app.world_mut()
        .entity_mut(working)
        .insert(Commitment::Working);

    app.update();

    for (entity, label) in [(carrying, "carrying"), (working, "working")] {
        let velocity = app
            .world()
            .entity(entity)
            .get::<VelocityComponent>()
            .unwrap()
            .value;
        assert!(
            velocity == Vec2::ZERO,
            "{label} bot must not receive a spread nudge; got {:?}",
            velocity
        );
    }
}

#[test]
fn spread_never_inserts_dmc_or_changes_commitment() {
    // The spread nudge is velocity-only. After many ticks of the full
    // chain the idle bot must still be `Commitment::Idle` and carry no
    // `DirectMovementComponent`, so the demand allocator grabs it
    // exactly as fast as before.
    let mut app = common::sim_app();
    paint(&mut app, IVec2::new(0, 0), IntentKind::Gather);
    paint(&mut app, IVec2::new(1, 0), IntentKind::Gather);
    let bot = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));

    for _ in 0..50 {
        app.update();
    }

    let world = app.world();
    let commitment = world
        .entity(bot)
        .get::<Commitment>()
        .expect("bot must keep its Commitment component");
    assert_eq!(
        *commitment,
        Commitment::Idle,
        "spread must not change Commitment"
    );
    assert!(
        world.entity(bot).get::<DirectMovementComponent>().is_none(),
        "spread must never insert a DirectMovementComponent"
    );
}

#[test]
fn stranded_idle_bot_drifts_toward_nearest_fit_cell() {
    // An idle bot whose current cell has no type-fit paint drifts
    // toward the nearest fit-paint cell. The bot starts on an
    // unpainted cell west of the only Gather cell, so the drift is
    // purely eastward and the bot must close the distance to the
    // Gather cell center over the run.
    let mut app = common::sim_app();
    let start = center(IVec2::new(-1, 0));
    let gather = IVec2::new(1, 0);
    paint(&mut app, gather, IntentKind::Gather);
    let bot = common::spawn_worker_at(&mut app, start);

    let target = center(gather);
    let initial_distance = start.distance(target);
    for _ in 0..150 {
        app.update();
    }

    let bot_pos = app
        .world()
        .entity(bot)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    assert!(
        bot_pos.x > start.x,
        "stranded bot must drift east toward the Gather cell; start.x={} got {:?}",
        start.x,
        bot_pos
    );
    let final_distance = bot_pos.distance(target);
    assert!(
        final_distance < initial_distance,
        "stranded bot must close the distance to the nearest fit cell; initial={} final={}",
        initial_distance,
        final_distance
    );
}

#[test]
fn stranded_bot_with_no_fit_paint_anywhere_stays_put() {
    // If no type-fit paint exists anywhere on the grid, the stranded
    // bot gets no nudge and stays put. This pins the "if no such cell
    // exists it does not move" rule.
    let mut app = spread_only_app();
    // No paint at all.
    let bot = common::spawn_worker_at(&mut app, center(IVec2::new(0, 0)));

    app.update();

    let velocity = app
        .world()
        .entity(bot)
        .get::<VelocityComponent>()
        .unwrap()
        .value;
    assert!(
        velocity == Vec2::ZERO,
        "stranded bot with no fit paint must not be nudged; got {:?}",
        velocity
    );
}
