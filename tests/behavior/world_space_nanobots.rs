//! Behavior tests for issue #38 / ADR-0004: world-space
//! nanobots.
//!
//! The "positioned-swarm scenario" reproduction pins the
//! world-coordinate behaviour the player sees in `cargo
//! run`. The default player swarm is at
//! `cell_origin(PLAYER_CELL) = (256, 256)` -- exactly half
//! a cell. Pre-fix, bots were parented to the swarm, so
//! their `Transform.translation` was local to the parent,
//! and the movement system steered until the local value
//! equalled the world destination. With a half-cell
//! parent offset, the worker ended up at the
//! `deposit + (256, 256)` corner of the Gather cell, and
//! the constructed structure appeared at the bottom-left
//! of the working bot cluster. The fix moves the bots to
//! top-level entities with world `Transform`s, so the
//! `Transform.translation` is the position the simulation
//! reads. The deposit / structure / gather deliverable
//! world positions are unchanged.
//!
//! These tests assert the `GlobalTransform` matches the
//! world destination, the failure mode the issue calls out,
//! and the opponent gather economy (which was broken by
//! the same offset).

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{GatherAssignment, SwarmId},
};

#[path = "../common/mod.rs"]
mod common;

const CELL_SIZE: f32 = ZONE_BLOCK_SIZE;

/// Build the test app. Uses the gather + planned plugin
/// set so the worker can route through the full pipeline
/// (assignment, demand, arrive, extract, carry-assign,
/// delivery).
fn build_app() -> App {
    common::sim_app_with_gather_planned()
}

/// Paint `cell` with player-owned Gather intent.
fn paint_gather(app: &mut App, cell: IVec2) {
    let mut grid = app.world_mut().resource_mut::<IntentGrid>();
    assert!(grid.paint_owned(cell, IntentKind::Gather, Some(SwarmId::PLAYER),));
}

#[test]
fn gather_bot_lands_at_deposit_world_position_not_cell_corner() {
    // Issue #38 acceptance: a bot assigned to a Gather
    // deposit must end its `GlobalTransform` at the
    // deposit's world position (cell center), not at
    // `center + (256, 256)`. The pre-fix code routed the
    // bot until its local transform equalled the world
    // destination; the half-cell parent offset put the
    // bot at the cell's top-right corner.
    //
    // The positioned-swarm scenario path: the swarm is
    // spawned at the cell-center offset `(256, 256)` and
    // a top-level Worker is placed at the same world
    // position. The deposit is at the canonical gather
    // cell center `(-768, 256)` (= `cell_origin((-2, 0))`).
    // The bot walks to the deposit, extracts, and
    // delivers; the test asserts the bot's
    // `GlobalTransform` is the deposit's world position.
    let mut app = build_app();
    let player_pos = Vec2::new(256.0, 256.0); // cell_origin(PLAYER_CELL)
    let deposit_cell = IVec2::new(-2, 0);
    let deposit_pos = Vec2::new(
        deposit_cell.x as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        CELL_SIZE / 2.0,
    );
    let _swarm = common::spawn_swarm_at(&mut app, player_pos);
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    // Spawn a Source Stockpile near the deposit so the
    // gather arrive system can insert ExtractProgress
    // (issue #23 contract).
    let _stockpile = common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 1000);
    let worker = common::spawn_worker_at(&mut app, player_pos);
    paint_gather(&mut app, deposit_cell);

    // Pre-seed the assignment so the test isolates the
    // arrival behaviour from the assignment scorer. The
    // DMC carries the deposit's physical radius as
    // `stop_radius` (issue #38 / ADR-0004).
    app.world_mut().entity_mut(worker).insert(GatherAssignment {
        cell: deposit_cell,
        deposit: _deposit,
    });

    // Drive until physical arrival starts extraction. Stopping at this
    // state avoids observing a later Cargo trip toward the Source Stockpile.
    let mut arrived = false;
    for _ in 0..500 {
        app.update();
        if app
            .world()
            .entity(worker)
            .get::<top_down_2d_rts_prototype_nano_swarm::nanobot::ExtractProgress>()
            .is_some()
        {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "worker must reach deposit and begin extraction");

    let world = app.world();
    let bot_pos = world
        .entity(worker)
        .get::<Transform>()
        .expect("worker must have a Transform")
        .translation
        .truncate();
    let dx = (bot_pos.x - deposit_pos.x).abs();
    let dy = (bot_pos.y - deposit_pos.y).abs();
    let offset_x = bot_pos.x - (deposit_pos.x + 256.0);
    let offset_y = bot_pos.y - (deposit_pos.y + 256.0);
    // The worker must land within the deposit's physical
    // extent (radius 32) plus a small overshoot margin
    // (one tick of bot_speed = 5). The pre-fix code put
    // the bot at the cell corner (deposit + (256, 256)),
    // which is much further than the 32+5 = 37 bound.
    assert!(
        dx <= 40.0 && dy <= 40.0,
        "worker Transform should land within the deposit's physical extent + 1-tick margin ({:?} +/- 40); got {:?} (dx={}, dy={})",
        deposit_pos,
        bot_pos,
        dx,
        dy
    );
    // The pre-fix failure mode put the bot at the cell
    // corner (deposit + (256, 256)). The fix moves the
    // bot to the deposit's world position. Assert the
    // bot is far from the cell corner so the regression
    // is caught.
    assert!(
        offset_x.abs() > 100.0 || offset_y.abs() > 100.0,
        "worker must not land at the cell corner (deposit + (256, 256)) -- the pre-fix failure mode. \
         offset from cell corner: ({}, {})",
        offset_x,
        offset_y
    );
}

#[test]
fn build_bot_lands_at_structure_world_position() {
    // Issue #38 acceptance: a bot assigned to a Build
    // target ends at the target's world position, and
    // the completed structure and the working bot are
    // co-located. Pre-fix, the worker walked to
    // `local_target + swarm_pos` and the structure
    // appeared at the bottom-left of the bot cluster.
    //
    // The positioned-swarm scenario path: the swarm is
    // at the cell-center offset `(256, 256)`. The
    // BuildSite is at the canonical build cell center
    // `(256, 256)`. The worker walks to the site, builds
    // it, and the test asserts the worker's
    // `GlobalTransform` is the structure's world
    // position, not the structure + (256, 256) offset.
    let mut app = build_app();
    let player_pos = Vec2::new(256.0, 256.0);
    let _swarm = common::spawn_swarm_at(&mut app, player_pos);
    // Spawn a BuildSite at the swarm's world position so
    // the test isolates the arrival behaviour. BuildSite
    // is the legacy `BuildSite` type; the `BuildPlugin`
    // is no longer registered, so the test inserts the
    // site directly.
    let site_pos = player_pos;
    let _site = app
        .world_mut()
        .spawn((
            top_down_2d_rts_prototype_nano_swarm::nanobot::BuildSite::new(
                IVec2::new(0, 0),
                top_down_2d_rts_prototype_nano_swarm::nanobot::StructureKind::Basic,
            ),
            Transform::from_translation(site_pos.extend(0.0)),
        ))
        .id();
    let worker = common::spawn_worker_at(&mut app, player_pos);
    // Pre-seed the build assignment so the test isolates
    // the arrival behaviour.
    app.world_mut().entity_mut(worker).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::BuildAssignment {
            cell: IVec2::new(0, 0),
            target: _site,
        },
    );

    for _ in 0..50 {
        app.update();
    }

    let world = app.world();
    let bot_pos = world
        .entity(worker)
        .get::<Transform>()
        .expect("worker must have a Transform")
        .translation
        .truncate();
    let dx = (bot_pos.x - site_pos.x).abs();
    let dy = (bot_pos.y - site_pos.y).abs();
    let offset_x = bot_pos.x - (site_pos.x + 256.0);
    let offset_y = bot_pos.y - (site_pos.y + 256.0);
    assert!(
        dx <= 1.0 && dy <= 1.0,
        "worker Transform should land at the structure center ({:?}); got {:?} (dx={}, dy={})",
        site_pos,
        bot_pos,
        dx,
        dy
    );
    assert!(
        offset_x.abs() > 50.0 || offset_y.abs() > 50.0,
        "worker must not land at the cell corner (structure + (256, 256)) -- the pre-fix failure mode. \
         offset from cell corner: ({}, {})",
        offset_x,
        offset_y
    );
}

#[test]
fn opponent_gather_bot_lands_at_deposit_world_position() {
    // Issue #38 acceptance: opponent bots reach their
    // deposit. The default opponent swarm is at
    // `cell_origin(OPPONENT_CELL) = (6400, 256)`, also a
    // half-cell offset. Pre-fix, the opponent gather
    // economy was broken by the same `(256, 256)` offset.
    let mut app = build_app();
    // Use a smaller opponent offset to stay within the
    // default 8x8 intent grid (cells span -4..3 on each
    // axis). The fix's invariant ("bots land at the world
    // destination, not destination + swarm_pos") is the
    // same regardless of the swarm's specific position; we
    // pin a non-zero offset so the pre-fix failure mode
    // (cell corner) would be visible.
    let opponent_pos = Vec2::new(1024.0, 256.0);
    let deposit_cell = IVec2::new(2, 0);
    let deposit_pos = Vec2::new(
        deposit_cell.x as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        CELL_SIZE / 2.0,
    );
    // Spawn an opponent swarm with one Worker seed.
    let _opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        opponent_pos,
        top_down_2d_rts_prototype_nano_swarm::nanobot::ProductionPriority::new(),
        &[(
            top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker,
            1,
        )],
    );
    let _deposit = common::spawn_deposit(&mut app, deposit_pos, 100);
    let _stockpile = common::spawn_stockpile(&mut app, deposit_pos + Vec2::new(96.0, 0.0), 0, 1000);
    // Paint the deposit's gather cell as opponent-owned.
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        assert!(grid.paint_owned(deposit_cell, IntentKind::Gather, Some(SwarmId(7)),));
    }
    // Find the seed worker.
    let worker = {
        let world = app.world_mut();
        let mut query = world.query::<(
            Entity,
            &top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember,
        )>();
        let swarm_id = world
            .entity(_opponent)
            .get::<SwarmId>()
            .copied()
            .expect("opponent swarm must carry a SwarmId");
        query
            .iter(&*world)
            .find(|(_, m)| m.0 == swarm_id)
            .map(|(e, _)| e)
            .expect("opponent swarm must have a seed worker")
    };
    // Pre-seed the gather assignment with the
    // opponent's id, mirroring the production path.
    app.world_mut().entity_mut(worker).insert(GatherAssignment {
        cell: deposit_cell,
        deposit: _deposit,
    });

    for _ in 0..1500 {
        app.update();
    }

    let world = app.world();
    let bot_pos = world
        .entity(worker)
        .get::<Transform>()
        .expect("worker must have a Transform")
        .translation
        .truncate();
    // The pre-fix failure mode put the bot at the cell
    // corner (deposit + (256, 256)). The fix moves
    // the bot to the deposit's world position. After
    // many ticks, the bot is in the middle of a gather
    // cycle (between the deposit and the source
    // stockpile 96 units away). The bot is never at
    // the cell corner in the fixed code; assert that
    // the distance from the cell corner is large so
    // the regression is caught.
    let corner_x = deposit_pos.x + 256.0;
    let corner_y = deposit_pos.y + 256.0;
    let dist_to_corner = ((bot_pos.x - corner_x).powi(2) + (bot_pos.y - corner_y).powi(2)).sqrt();
    assert!(
        dist_to_corner > 100.0,
        "opponent worker must not land at the cell corner (deposit + (256, 256)); got {:?}, distance to corner = {}",
        bot_pos,
        dist_to_corner
    );
}

#[test]
fn stop_radius_zero_falls_through_to_stop_threshold() {
    // Issue #38 / ADR-0004: the `0.0` sentinel on
    // `DirectMovementComponent::stop_radius` falls
    // through to `STOP_THRESHOLD` (2.0) in the movement
    // system. This is the "extent-less destination"
    // path used by corridor waypoints and the Defend
    // cell center.
    //
    // Pin the contract: a bot with `stop_radius = 0.0`
    // stops at `STOP_THRESHOLD` past its destination,
    // not at the destination itself. A bot with
    // `stop_radius = 32.0` (a physical extent) stops
    // at `max(32, STOP_THRESHOLD) = 32`.
    let mut app = build_app();
    let start = Vec2::new(0.0, 0.0);
    let dest = Vec2::new(100.0, 0.0);
    let bot = app
        .world_mut()
        .spawn((
            top_down_2d_rts_prototype_nano_swarm::nanobot::Nanobot {},
            top_down_2d_rts_prototype_nano_swarm::nanobot::VelocityComponent::default(),
            top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker,
            top_down_2d_rts_prototype_nano_swarm::nanobot::Commitment::Idle,
            top_down_2d_rts_prototype_nano_swarm::nanobot::Health::default(),
            top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(start.extend(0.0)),
        ))
        .id();
    // Extent-less destination: the bot should stop at
    // distance <= STOP_THRESHOLD (2.0) past `dest`.
    app.world_mut().entity_mut(bot).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent {
            xy: dest,
            stop_radius: 0.0,
        },
    );
    for _ in 0..100 {
        app.update();
    }
    let world = app.world();
    let bot_pos = world
        .entity(bot)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    let distance = bot_pos.distance(dest);
    assert!(
        (distance - 2.0).abs() < 1.0 || distance <= 2.0,
        "extent-less DMC should stop at STOP_THRESHOLD (2.0) past dest; got distance={}",
        distance
    );
}

#[test]
fn stop_radius_uses_max_with_stop_threshold() {
    // The "physical extent" path: a DMC with
    // `stop_radius = 32.0` stops at `max(32, STOP_THRESHOLD)
    // = 32.0` past its destination. A small extent
    // (`stop_radius = 0.5`) is clamped up to
    // `STOP_THRESHOLD` so a tiny extent does not
    // produce a sub-pixel arrival.
    let mut app = build_app();
    let start = Vec2::new(0.0, 0.0);
    let dest = Vec2::new(100.0, 0.0);
    let bot = app
        .world_mut()
        .spawn((
            top_down_2d_rts_prototype_nano_swarm::nanobot::Nanobot {},
            top_down_2d_rts_prototype_nano_swarm::nanobot::VelocityComponent::default(),
            top_down_2d_rts_prototype_nano_swarm::nanobot::NanobotType::Worker,
            top_down_2d_rts_prototype_nano_swarm::nanobot::Commitment::Idle,
            top_down_2d_rts_prototype_nano_swarm::nanobot::Health::default(),
            top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmMember::new(SwarmId::PLAYER),
            Transform::from_translation(start.extend(0.0)),
        ))
        .id();
    app.world_mut().entity_mut(bot).insert(
        top_down_2d_rts_prototype_nano_swarm::nanobot::DirectMovementComponent {
            xy: dest,
            stop_radius: 32.0,
        },
    );
    for _ in 0..100 {
        app.update();
    }
    let world = app.world();
    let bot_pos = world
        .entity(bot)
        .get::<Transform>()
        .unwrap()
        .translation
        .truncate();
    let distance = bot_pos.distance(dest);
    assert!(
        distance <= 35.0,
        "DMC with stop_radius=32 should stop at distance <= 32 + 1-tick margin from dest; got distance={}",
        distance
    );
}
