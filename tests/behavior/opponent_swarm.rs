//! Opponent swarms use the same intent, demand, and production systems.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    ZONE_BLOCK_SIZE,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Commitment, NanobotType, OpponentSwarm, PRODUCTION_TICKS_PER_BOT, PrepaintedIntent,
        ProductionFacility, SeedNanobots, SoftWorkSlots, Swarm, SwarmId, SwarmMember,
        best_candidate, spawn_opponent_swarm,
    },
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn opponent_initialization_prepaints_owned_intent_and_seeds_owned_bots() {
    let mut app = common::sim_app_with_production();
    let cell = IVec2::new(2, 0);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        Vec2::new(2_000.0, 0.0),
        &[PrepaintedIntent::new(cell, IntentKind::Gather)],
        &[SeedNanobots::new(NanobotType::Worker, 2)],
    );
    let id = *app.world().get::<SwarmId>(opponent).unwrap();

    assert!(app.world().get::<OpponentSwarm>(opponent).is_some());
    assert!(app.world().get::<Swarm>(opponent).is_some());
    assert!(
        app.world()
            .resource::<IntentGrid>()
            .cell(cell)
            .unwrap()
            .has(IntentKind::Gather)
    );
    let world = app.world_mut();
    let workers = world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, member)| **kind == NanobotType::Worker && member.0 == id)
        .count();
    assert_eq!(workers, 2);
}

#[test]
fn opponent_bot_reads_its_prepainted_intent_through_shared_scoring() {
    let mut app = common::sim_app_with_production();
    let cell = IVec2::new(2, 0);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        Vec2::new(2_000.0, 0.0),
        &[PrepaintedIntent::new(cell, IntentKind::Gather)],
        &[SeedNanobots::new(NanobotType::Worker, 1)],
    );
    app.update();
    let id = *app.world().get::<SwarmId>(opponent).unwrap();
    let picked = best_candidate(
        app.world().resource::<IntentGrid>(),
        NanobotType::Worker,
        Commitment::Idle,
        Vec2::new(1.5 * ZONE_BLOCK_SIZE, 0.5 * ZONE_BLOCK_SIZE),
        app.world().resource::<SoftWorkSlots>(),
        ZONE_BLOCK_SIZE,
        &IntentKind::ALL,
        id,
    )
    .expect("opponent worker sees owned intent");
    assert_eq!((picked.cell, picked.kind), (cell, IntentKind::Gather));
}

#[test]
fn opponent_workload_demand_drives_opponent_owned_production() {
    let mut app = common::sim_app_with_production();
    let cell = IVec2::new(2, 0);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        common::cell_world_center(cell),
        &[PrepaintedIntent::new(cell, IntentKind::Gather)],
        &[],
    );
    let opponent_id = *app.world().get::<SwarmId>(opponent).unwrap();
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: common::cell_world_center(cell),
            amount: 1_000,
            capacity: 1_000,
            radius: 32.0,
        },
    );
    let facility = common::spawn_facility_at(&mut app, opponent, common::cell_world_center(cell));

    app.update();
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        Some(NanobotType::Worker),
    );
    for _ in 0..PRODUCTION_TICKS_PER_BOT {
        app.update();
    }

    let world = app.world_mut();
    let produced = world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, member)| **kind == NanobotType::Worker && member.0 == opponent_id)
        .count();
    assert_eq!(produced, 1);
}
