//! Demand-driven Production Facility behavior.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        NanobotType, OwnerSwarm, PRODUCTION_COST_PER_BOT, PRODUCTION_TICKS_PER_BOT,
        PopulationDemand, ProductionFacility, SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure,
        StructureKind, SwarmId, SwarmMember,
    },
    resources::{ResourceKind, ResourceLedger},
};

#[path = "../common/mod.rs"]
mod common;

fn add_gather_work(app: &mut App, cell: IVec2) {
    assert!(app.world_mut().resource_mut::<IntentGrid>().paint(
        cell,
        IntentKind::Gather,
        SwarmId::PLAYER
    ));
    common::spawn_deposit(
        app,
        common::DepositFixture {
            world_pos: common::cell_world_center(cell),
            amount: 1_000,
            capacity: 1_000,
            radius: 32.0,
        },
    );
}

#[test]
fn facility_commits_to_real_workload_demand_and_consumes_its_hopper() {
    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::ZERO);
    let input_before = app
        .world()
        .get::<ProductionFacility>(facility)
        .unwrap()
        .input_amount;
    let ledger_before = app
        .world()
        .resource::<ResourceLedger>()
        .total_for(SwarmId::PLAYER, ResourceKind::Minerals);

    app.update();

    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(state.current_target, Some(NanobotType::Worker));
    assert_eq!(input_before - state.input_amount, PRODUCTION_COST_PER_BOT);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        ledger_before - PRODUCTION_COST_PER_BOT,
    );
}

#[test]
fn funded_cycle_retains_its_type_and_produces_an_owned_nanobot() {
    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::ZERO);
    app.update();
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        Some(NanobotType::Worker),
    );

    app.world_mut().resource_mut::<IntentGrid>().erase(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    for _ in 0..PRODUCTION_TICKS_PER_BOT {
        app.update();
    }

    let world = app.world_mut();
    let produced = world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, member)| **kind == NanobotType::Worker && member.0 == SwarmId::PLAYER)
        .count();
    assert_eq!(produced, 1, "funded work completes after demand disappears");
}

#[test]
fn missing_owner_is_not_treated_as_the_player() {
    let mut app = common::sim_app_with_production();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let mut facility = ProductionFacility::new();
    facility.input_amount = facility.input_capacity;
    let entity = app.world_mut().spawn(facility).id();
    app.update();
    let state = app.world().get::<ProductionFacility>(entity).unwrap();
    assert_eq!(state.current_target, None);
    assert_eq!(state.input_amount, state.input_capacity);
}

#[test]
fn owner_must_reference_a_real_swarm() {
    let mut app = common::sim_app_with_production();
    common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let invalid_owner = app.world_mut().spawn(SwarmId::PLAYER).id();
    let mut facility = ProductionFacility::new();
    facility.input_amount = facility.input_capacity;
    let entity = app
        .world_mut()
        .spawn((facility, OwnerSwarm(invalid_owner)))
        .id();

    app.update();

    let state = app.world().get::<ProductionFacility>(entity).unwrap();
    assert_eq!(state.current_target, None);
    assert_eq!(state.input_amount, state.input_capacity);
}

#[test]
fn damaged_facility_does_not_commit_until_operational() {
    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::ZERO);
    let mut condition = Structure::new(StructureKind::Basic);
    condition.health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1;
    app.world_mut().entity_mut(facility).insert(condition);
    app.update();
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        None
    );

    app.world_mut()
        .get_mut::<Structure>(facility)
        .unwrap()
        .health = SUPPORT_OPERATIONAL_HEALTH_THRESHOLD;
    app.update();
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        Some(NanobotType::Worker),
    );
}

#[test]
fn funded_cycles_count_as_coverage_for_other_facilities() {
    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    for x in 0..4 {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::new(x, 0),
            IntentKind::Corridor,
            SwarmId::PLAYER,
        );
    }
    common::spawn_defender_at(&mut app, Vec2::ZERO);
    let producing = common::spawn_facility_at(&mut app, swarm, Vec2::ZERO);
    app.world_mut()
        .get_mut::<ProductionFacility>(producing)
        .unwrap()
        .current_target = Some(NanobotType::Defender);
    let idle = common::spawn_facility_at(&mut app, swarm, Vec2::new(100.0, 0.0));
    app.update();

    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Defender),
        2,
    );
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(idle)
            .unwrap()
            .current_target,
        None
    );
}

#[test]
fn empty_hopper_retries_after_delivery_without_block_state() {
    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let facility = app
        .world_mut()
        .spawn((ProductionFacility::new(), OwnerSwarm(swarm)))
        .id();
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        None
    );

    common::fill_facility_input(&mut app, facility);
    app.update();
    assert_eq!(
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .current_target,
        Some(NanobotType::Worker),
    );
}
