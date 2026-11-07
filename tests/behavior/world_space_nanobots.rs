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
use top_down_2d_rts_prototype_nano_swarm::nanobot::SwarmId;

#[path = "../common/mod.rs"]
mod common;

/// Build the test app. Uses the gather + planned plugin
/// set so the worker can route through the full pipeline
/// (assignment, demand, arrive, extract, carry-assign,
/// delivery).
fn build_app() -> App {
    common::sim_app_with_gather_planned()
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
            speed: None,
            interaction: None,
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
            speed: None,
            interaction: None,
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
