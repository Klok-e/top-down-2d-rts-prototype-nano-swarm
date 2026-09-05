//! Scenario tests for issue #16: Production Collapse
//! win/loss detection.
//!
//! Each test isolates one recoverable / collapsed state and
//! asserts the corresponding
//! [`ProductionCollapseState`] flag flips. The tests build the
//! smallest Bevy `App` that proves the system wiring:
//! swarms, facilities, and the production + collapse systems
//! chained in order.

use std::f32::consts::TAU;

use bevy::{math::Vec2, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        Cargo, CollapsePlugin, GatherAssignment, GatherPlugin, HaulerAssignment,
        LogisticsReservation, MatchOutcome, NanobotType, OpponentSwarm, OwnerSwarm,
        PRODUCTION_COST_PER_BOT, PRODUCTION_TICKS_PER_BOT, PlannedKind, PlannedStructure,
        PlannedStructurePlugin, ProductionCollapseState, ProductionFacility, ProductionPlugin,
        ProductionPriority, RecoveryFacts, SOURCE_STOCKPILE_PLACEMENT_COUNT,
        SOURCE_STOCKPILE_PLACEMENT_RADIUS, Swarm, SwarmId, SwarmMember, SwarmProduction,
        evaluate_recovery,
    },
    resources::{ResourceKind, ResourceLedger, Stockpile, StockpileRole},
};
#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    // Empty global priority by default; each test sets the
    // priorities it needs.
    let mut app = common::sim_app_with_collapse();
    app.insert_resource(ProductionPriority::new());
    app
}

fn build_planning_app() -> App {
    let mut app = common::sim_app();
    app.add_plugins(PlannedStructurePlugin)
        .add_plugins(ProductionPlugin)
        .add_plugins(CollapsePlugin)
        .insert_resource(ProductionPriority::new());
    app
}

fn build_gather_recovery_app() -> App {
    let mut app = common::sim_app();
    app.add_plugins(GatherPlugin)
        .add_plugins(PlannedStructurePlugin)
        .add_plugins(ProductionPlugin)
        .add_plugins(CollapsePlugin)
        .insert_resource(ProductionPriority::new());
    app
}

#[test]
fn player_swarm_with_working_facility_is_not_collapsed() {
    // The player swarm has 1 facility that is currently
    // producing, plus a Worker + Hauler. The collapse
    // system must report no collapse.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );
    let _pile = common::spawn_stockpile(&mut app, player_pos, PRODUCTION_COST_PER_BOT * 5, 1000);
    let facility = common::spawn_facility_at(&mut app, player, player_pos);

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        !state.player_collapsed,
        "a working facility means the player swarm is not collapsed"
    );
    assert!(!state.opponent_collapsed);
    // The facility should have picked a target by now.
    let f = app
        .world()
        .entity(facility)
        .get::<ProductionFacility>()
        .unwrap();
    assert!(
        f.is_busy(),
        "facility should have started a production cycle"
    );
}

#[test]
fn busy_unowned_facility_uses_player_fallback_for_collapse_detection() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 1);
    }
    app.world_mut().spawn((
        Swarm {},
        SwarmId(1),
        OpponentSwarm {},
        SwarmProduction::new(ProductionPriority::new()),
        Transform::from_translation(Vec3::X),
    ));
    common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[]);
    let mut production = ProductionFacility::new();
    production.current_target = Some(NanobotType::Worker);
    production.progress = PRODUCTION_TICKS_PER_BOT - 1;
    app.world_mut()
        .spawn((production, Transform::from_translation(Vec3::ZERO)));

    app.update();

    let produced_members = app
        .world_mut()
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(app.world())
        .map(|(kind, member)| (*kind, member.0))
        .collect::<Vec<_>>();
    assert_eq!(
        produced_members,
        vec![(NanobotType::Worker, SwarmId::PLAYER)],
        "an unowned facility must complete production for the player even when an opponent was spawned first",
    );
    assert!(
        !app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "collapse detection must use the same player fallback as production",
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress,
    );
}

#[test]
fn unfunded_unowned_facility_is_not_a_player_hauler_destination() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().spawn((
        ProductionFacility::new(),
        Transform::from_translation(Vec3::ZERO),
    ));
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: PRODUCTION_COST_PER_BOT,
            capacity: 100,
            radius: 32.0,
        },
        OwnerSwarm(player),
        StockpileRole::Sink,
        Transform::from_translation(Vec3::ZERO),
    ));

    app.update();

    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "an empty unowned facility cannot receive player Hauler deliveries",
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Defeat,
    );
}

#[test]
fn player_swarm_with_no_facility_and_recoverable_crew_is_not_collapsed() {
    // The player swarm lost every facility but still has
    // 1 Worker and 1 Hauler. The collapse system must
    // report "can recover, not collapsed".
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(1, 0),
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    let stockpile = common::spawn_stockpile(&mut app, player_pos, PRODUCTION_COST_PER_BOT, 100);
    app.world_mut()
        .entity_mut(stockpile)
        .insert(OwnerSwarm(player));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        !state.player_collapsed,
        "a recoverable crew means the player swarm is not collapsed"
    );
    assert!(!state.opponent_collapsed);
    assert!(!state.player_won());
}

#[test]
fn unowned_stockpile_material_does_not_preserve_player_recovery() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(1, 0),
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    common::spawn_stockpile(&mut app, Vec2::ZERO, PRODUCTION_COST_PER_BOT, 100);

    app.update();

    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "Haulers cannot use an unowned endpoint as a production recovery path",
    );
}

#[test]
fn stranded_hauler_cargo_does_not_prevent_collapse() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(1, 0),
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    let hauler = {
        let world = app.world_mut();
        let mut nanobots = world.query::<(Entity, &NanobotType)>();
        nanobots
            .iter(world)
            .find_map(|(entity, kind)| (*kind == NanobotType::Hauler).then_some(entity))
            .expect("the recovery crew includes a Hauler")
    };
    app.world_mut().entity_mut(hauler).insert(Cargo {
        kind: ResourceKind::Minerals,
        amount: PRODUCTION_COST_PER_BOT,
    });
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );

    app.update();

    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "cargo without a logistics assignment cannot preserve a recovery path",
    );
}

#[test]
fn assigned_hauler_cargo_preserves_a_recovery_path() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(1, 0),
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    let source = common::spawn_stockpile(&mut app, Vec2::ZERO, 0, 100);
    app.world_mut()
        .entity_mut(source)
        .insert(OwnerSwarm(player));
    let missing_sink = app.world_mut().spawn_empty().id();
    app.world_mut().despawn(missing_sink);
    let hauler = {
        let world = app.world_mut();
        let mut nanobots = world.query::<(Entity, &NanobotType)>();
        nanobots
            .iter(world)
            .find_map(|(entity, kind)| (*kind == NanobotType::Hauler).then_some(entity))
            .expect("the recovery crew includes a Hauler")
    };
    let mut reservation = LogisticsReservation::new(
        source,
        missing_sink,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );
    reservation.source_remaining = 0;
    app.world_mut().entity_mut(hauler).insert((
        Cargo {
            kind: ResourceKind::Minerals,
            amount: PRODUCTION_COST_PER_BOT,
        },
        HaulerAssignment {
            source,
            sink: missing_sink,
        },
        reservation,
    ));

    app.update();

    assert!(
        !app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "assigned cargo from a live Source can reach rebuilt Sink infrastructure",
    );
}

#[test]
fn partial_facility_input_and_complementary_staged_material_are_recoverable() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    let mut facility = ProductionFacility::new();
    facility.input_amount = PRODUCTION_COST_PER_BOT - 1;
    app.world_mut().spawn((
        facility,
        OwnerSwarm(player),
        Transform::from_translation(Vec2::ZERO.extend(0.0)),
    ));
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 1,
            capacity: 100,
            radius: 32.0,
        },
        OwnerSwarm(player),
        StockpileRole::Sink,
        Transform::from_translation(Vec2::ZERO.extend(0.0)),
    ));

    app.update();

    assert!(
        !app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "partial hopper input plus deliverable staged material is a complete recovery path",
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress,
    );
}

#[test]
fn partial_facility_input_and_source_only_material_are_not_recoverable_without_build_space() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    let mut facility = ProductionFacility::new();
    facility.input_amount = PRODUCTION_COST_PER_BOT - 1;
    app.world_mut().spawn((
        facility,
        OwnerSwarm(player),
        Transform::from_translation(Vec2::ZERO.extend(0.0)),
    ));
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 1,
            capacity: 100,
            radius: 32.0,
        },
        OwnerSwarm(player),
        Transform::from_translation(Vec2::ZERO.extend(0.0)),
    ));

    app.update();

    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "Source material cannot bypass the missing Sink logistics leg",
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Defeat
    );
}

#[test]
fn occupied_facility_build_cell_can_plan_its_local_sink_recovery_path() {
    let mut app = build_planning_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let cell = IVec2::ZERO;
    let facility_pos = common::cell_world_center(cell);
    let player = common::spawn_swarm_at(&mut app, facility_pos);
    common::spawn_worker_at(&mut app, facility_pos + Vec2::new(-144.0, -144.0));
    common::spawn_hauler_at(&mut app, facility_pos + Vec2::new(-144.0, 0.0));
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(cell, IntentKind::Build, SwarmId::PLAYER);
    app.world_mut().spawn((
        ProductionFacility::new(),
        OwnerSwarm(player),
        Transform::from_translation(facility_pos.extend(0.0)),
    ));
    let source = common::spawn_stockpile(
        &mut app,
        common::cell_world_center(IVec2::new(-2, 0)),
        PRODUCTION_COST_PER_BOT,
        100,
    );
    app.world_mut()
        .entity_mut(source)
        .insert(OwnerSwarm(player));

    for _ in 0..100 {
        app.update();
        if app
            .world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .any(|plan| plan.kind == PlannedKind::SinkStockpile)
        {
            break;
        }
    }

    assert!(
        app.world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .any(|planned| planned.kind == PlannedKind::SinkStockpile),
        "the Sink planner can place beside a facility in its occupied Build cell",
    );
    assert!(
        !app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "the actual local Sink plan preserves recovery",
    );
}

#[test]
fn remote_build_space_cannot_supply_an_idle_facility_outside_build_paint() {
    let mut app = build_planning_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let facility_pos = common::cell_world_center(IVec2::ZERO);
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        facility_pos,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(3, 0),
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    app.world_mut().spawn((
        ProductionFacility::new(),
        OwnerSwarm(player),
        Transform::from_translation(facility_pos.extend(0.0)),
    ));
    let source = common::spawn_stockpile(
        &mut app,
        facility_pos + Vec2::new(-200.0, 0.0),
        PRODUCTION_COST_PER_BOT,
        100,
    );
    app.world_mut()
        .entity_mut(source)
        .insert(OwnerSwarm(player));

    app.update();

    assert!(
        !app.world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .any(|planned| planned.kind == PlannedKind::SinkStockpile),
        "remote Build paint cannot create a Sink for an unpainted facility",
    );
    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "unreachable remote Build space cannot prevent collapse",
    );
}

#[test]
fn blocked_source_ring_cannot_turn_gather_paint_into_a_material_path() {
    let mut app = build_gather_recovery_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let gather_cell = IVec2::ZERO;
    let deposit_pos = common::cell_world_center(gather_cell);
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        deposit_pos,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    app.world_mut().resource_mut::<IntentGrid>().paint(
        gather_cell,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    let deposit = common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: deposit_pos,
            amount: 100,
            capacity: 1000,
            radius: 32.0,
        },
    );
    let facility_pos = common::cell_world_center(IVec2::new(3, 0));
    app.world_mut().spawn((
        ProductionFacility::new(),
        OwnerSwarm(player),
        Transform::from_translation(facility_pos.extend(0.0)),
    ));
    let sink = common::spawn_sink_stockpile(&mut app, facility_pos, 0, 100);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(player));

    let foreign_owner = app.world_mut().spawn_empty().id();
    for index in 0..SOURCE_STOCKPILE_PLACEMENT_COUNT {
        let angle = index as f32 * (TAU / SOURCE_STOCKPILE_PLACEMENT_COUNT as f32);
        let position =
            deposit_pos + Vec2::new(angle.cos(), angle.sin()) * SOURCE_STOCKPILE_PLACEMENT_RADIUS;
        app.world_mut().spawn((
            ProductionFacility::new(),
            OwnerSwarm(foreign_owner),
            Transform::from_translation(position.extend(0.0)),
        ));
    }
    let worker = {
        let world = app.world_mut();
        let mut nanobots = world.query::<(Entity, &NanobotType)>();
        nanobots
            .iter(world)
            .find_map(|(entity, kind)| (*kind == NanobotType::Worker).then_some(entity))
            .expect("the recovery crew includes a Worker")
    };
    app.world_mut()
        .entity_mut(worker)
        .insert(GatherAssignment::new(gather_cell, deposit));

    app.update();

    assert!(
        !app.world_mut()
            .query::<&PlannedStructure>()
            .iter(app.world())
            .any(|planned| planned.kind == PlannedKind::SourceStockpile),
        "the exact Source planner rejects every obstructed ring candidate",
    );
    assert!(
        app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "blocked Gather extraction cannot preserve production recovery",
    );
}

#[test]
fn player_swarm_with_no_facility_and_no_haulers_is_collapsed() {
    // The player swarm lost its facility and has no
    // haulers. A lone worker cannot deliver minerals to a
    // stockpile, so the production chain is dead. The
    // collapse system must report a player loss.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.player_collapsed,
        "no facility + no haulers means the player swarm is collapsed"
    );
    assert!(!state.opponent_collapsed);
    assert!(state.player_lost());
    assert!(!state.player_won());
}

#[test]
fn player_swarm_with_no_facility_and_no_workers_is_collapsed() {
    // Mirror of the previous test: only haulers remain.
    // They cannot extract from deposits, so the production
    // chain is dead.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Hauler, 2), (NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(state.player_collapsed);
    assert!(state.player_lost());
}

#[test]
fn opponent_swarm_with_no_facility_and_no_haulers_means_player_wins() {
    // The opponent swarm is the one that lost its
    // production capacity; the player swarm is healthy.
    // The collapse system must report a player win.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );
    common::spawn_facility_at(&mut app, player, player_pos);

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_priority = ProductionPriority::new();
    opponent_priority.set_weight(NanobotType::Worker, 5);
    opponent_priority.set_weight(NanobotType::Hauler, 2);
    let _opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        opponent_pos,
        opponent_priority,
        &[(NanobotType::Worker, 2), (NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.opponent_collapsed,
        "opponent with no facility + no haulers must be collapsed"
    );
    assert!(!state.player_collapsed);
    assert!(
        state.player_won(),
        "player wins when only the opponent collapses"
    );
    assert!(!state.player_lost());
}

#[test]
fn both_swarms_collapsed_is_a_loss_not_a_win() {
    // Degenerate scenario: both sides lose their
    // production capacity. The player_lost flag takes
    // priority over player_won so the UI shows the loss
    // state.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player =
        common::spawn_swarm_with_nanobots(&mut app, player_pos, &[(NanobotType::Defender, 1)]);

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_priority = ProductionPriority::new();
    opponent_priority.set_weight(NanobotType::Worker, 5);
    let _opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        opponent_pos,
        opponent_priority,
        &[(NanobotType::Defender, 1)],
    );

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(state.player_collapsed);
    assert!(state.opponent_collapsed);
    assert!(state.player_lost());
    assert!(!state.player_won(), "mutual collapse is a loss, not a win");
}

#[test]
fn swarm_at_production_target_is_not_collapsed_without_a_facility() {
    // A swarm that has reached its production priority target
    // has no unmet demand. "No facility" is the success
    // state, not the collapse state.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let _player =
        common::spawn_swarm_with_nanobots(&mut app, player_pos, &[(NanobotType::Worker, 2)]);

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        !state.player_collapsed,
        "a swarm at target is at rest, not collapsed"
    );
}

#[test]
fn collapse_state_updates_after_facility_is_destroyed() {
    // Dynamic scenario: the player starts healthy with a
    // working facility, then the facility is despawned.
    // After the next tick, the system must report a
    // collapse (assuming the swarm cannot recover on its
    // own -- here the swarm has no crew at all).
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    // No nanobots; the swarm is essentially empty.
    let player = app
        .world_mut()
        .spawn((
            Swarm {},
            Transform::from_translation(player_pos.extend(0.0)),
        ))
        .id();
    let facility = common::spawn_facility_at(&mut app, player, player_pos);
    let _pile = common::spawn_stockpile(&mut app, player_pos, PRODUCTION_COST_PER_BOT * 5, 1000);

    // Tick once to let the production system start the
    // facility.
    app.update();
    {
        let f = app
            .world()
            .entity(facility)
            .get::<ProductionFacility>()
            .unwrap();
        assert!(f.is_busy(), "facility must be busy after one tick");
    }
    let state = app.world().resource::<ProductionCollapseState>();
    assert!(!state.player_collapsed, "facility is busy, no collapse yet");

    // Destroy the facility. Production still has unmet
    // demand but the swarm has no nanobots to recover.
    app.world_mut().despawn(facility);
    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.player_collapsed,
        "no facility + no nanobots means a player collapse"
    );
    assert!(state.player_lost());
}

#[test]
fn opponent_with_recoverable_crew_does_not_trigger_player_win() {
    // Opponent lost its facility but still has a Worker
    // and a Hauler. The opponent is not collapsed, so the
    // player has not won yet.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[(NanobotType::Worker, 2), (NanobotType::Hauler, 1)],
    );
    common::spawn_facility_at(&mut app, player, player_pos);

    let opponent_pos = Vec2::new(2000.0, 0.0);
    let mut opponent_priority = ProductionPriority::new();
    opponent_priority.set_weight(NanobotType::Worker, 5);
    opponent_priority.set_weight(NanobotType::Hauler, 2);
    let opponent = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        opponent_pos,
        opponent_priority,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    let opponent_id = *app
        .world()
        .entity(opponent)
        .get::<SwarmId>()
        .expect("opponent swarm has identity");
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(2, 0),
        IntentKind::Build,
        opponent_id,
    );
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        opponent_id,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: PRODUCTION_COST_PER_BOT,
            capacity: 100,
            radius: 32.0,
        },
        OwnerSwarm(opponent),
        Transform::from_translation(opponent_pos.extend(0.0)),
    ));

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(!state.opponent_collapsed);
    assert!(!state.player_won());
    assert!(!state.player_lost());
}

#[test]
fn idle_facility_with_no_stockpile_is_not_working_for_collapse_check() {
    // A facility exists but cannot start a production
    // cycle (no stockpile, no material). It is idle and
    // must not count as "working production". The swarm
    // has only Defenders, so the collapse system must
    // report a player loss.
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player_pos = Vec2::new(0.0, 0.0);
    let player =
        common::spawn_swarm_with_nanobots(&mut app, player_pos, &[(NanobotType::Defender, 2)]);
    // Spawn the facility directly (not via the filling helper)
    // so its input hopper is empty: without material the facility
    // cannot start a cycle and must not count as working
    // production.
    let _facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(player),
            Transform::from_translation(player_pos.extend(0.0)),
        ))
        .id();

    app.update();

    let state = app.world().resource::<ProductionCollapseState>();
    assert!(
        state.player_collapsed,
        "idle facility + no recover crew must register as a player collapse"
    );
}

#[test]
fn worker_and_hauler_without_rebuild_path_are_unrecoverable() {
    let outcome = evaluate_recovery(RecoveryFacts {
        has_unmet_demand: true,
        operational_production: false,
        viable_planned_facility: false,
        recoverable_existing_facility: false,
        funded_existing_facility: false,
        has_worker: true,
        has_hauler: true,
        has_build_space: false,
        has_material_path: false,
        existing_facility_material_path: false,
    });

    assert!(outcome.collapsed);
}

#[test]
fn supplied_existing_facility_is_a_recovery_path_without_build_space() {
    let outcome = evaluate_recovery(RecoveryFacts {
        has_unmet_demand: true,
        recoverable_existing_facility: true,
        has_worker: true,
        has_hauler: true,
        has_material_path: true,
        ..Default::default()
    });

    assert!(!outcome.collapsed);
}

#[test]
fn paid_existing_cycle_needs_only_a_repair_worker() {
    let outcome = evaluate_recovery(RecoveryFacts {
        has_unmet_demand: true,
        funded_existing_facility: true,
        has_worker: true,
        ..Default::default()
    });

    assert!(!outcome.collapsed);
}

#[test]
fn stockpile_in_only_build_cell_leaves_free_space_for_production_recovery() {
    let mut app = build_app();
    {
        let mut priority = app.world_mut().resource_mut::<ProductionPriority>();
        priority.set_weight(NanobotType::Worker, 5);
        priority.set_weight(NanobotType::Hauler, 2);
    }
    let player = common::spawn_swarm_with_nanobots(
        &mut app,
        Vec2::ZERO,
        &[(NanobotType::Worker, 1), (NanobotType::Hauler, 1)],
    );
    let build_cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        build_cell,
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    let stockpile = common::spawn_stockpile(
        &mut app,
        common::cell_world_center(build_cell),
        PRODUCTION_COST_PER_BOT,
        100,
    );
    app.world_mut()
        .entity_mut(stockpile)
        .insert(OwnerSwarm(player));
    app.world_mut().resource_mut::<ResourceLedger>().add_for(
        SwarmId::PLAYER,
        ResourceKind::Minerals,
        PRODUCTION_COST_PER_BOT,
    );

    app.update();

    assert!(
        !app.world()
            .resource::<ProductionCollapseState>()
            .player_collapsed,
        "a stockpile occupies only its footprint: the remaining Build space, crew, and materials allow rebuilding",
    );
}
