//! Demand-driven Production Facility behavior.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::tuning::Tuning;
use top_down_2d_rts_prototype_nano_swarm::{
    battle_statistics::BattleCounters,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        NanobotType, OwnerSwarm, PopulationDemand, ProductionFacility,
        SUPPORT_OPERATIONAL_HEALTH_THRESHOLD, Structure, StructureKind, SwarmId, SwarmMember,
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
    app.init_resource::<BattleCounters>();
    app.world_mut()
        .resource_mut::<Tuning>()
        .production
        .cost_per_bot = 27;
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
    assert_eq!(input_before - state.input_amount, 27);
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total_for(SwarmId::PLAYER, ResourceKind::Minerals),
        ledger_before - 27,
    );
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .minerals_consumed,
        27u64
    );
}

#[test]
fn funded_cycle_retains_its_type_and_produces_an_owned_nanobot() {
    let mut app = common::sim_app_with_production();
    app.init_resource::<BattleCounters>();
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
    for _ in 0..Tuning::default().production.ticks_per_bot {
        app.update();
    }

    let world = app.world_mut();
    let produced = world
        .query::<(&NanobotType, &SwarmMember)>()
        .iter(world)
        .filter(|(kind, member)| **kind == NanobotType::Worker && member.0 == SwarmId::PLAYER)
        .count();
    assert_eq!(produced, 1, "funded work completes after demand disappears");
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .births,
        1
    );
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

#[test]
fn starved_facility_with_unmet_demand_offers_a_queued_cycle_to_haulers() {
    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    add_gather_work(&mut app, IVec2::ZERO);
    let facility = app
        .world_mut()
        .spawn((ProductionFacility::new(), OwnerSwarm(swarm)))
        .id();
    let tuning = app.world().resource::<Tuning>().clone();

    app.update();

    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(
        state.current_target, None,
        "an empty hopper cannot fund a cycle"
    );
    assert_eq!(
        state.claimable_space(&tuning),
        state.input_capacity + tuning.production.cost_per_bot,
        "unmet demand lets haulers queue the cycle after the one being funded",
    );

    app.world_mut().resource_mut::<IntentGrid>().erase(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    app.update();

    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(
        state.claimable_space(&tuning),
        state.input_capacity,
        "covered demand withdraws the queued cycle",
    );
}

/// Run one regional allocation period, after which delivery windows are fresh.
fn run_allocation_period(app: &mut App) {
    for _ in 0..7 {
        app.update();
    }
}

#[test]
fn delivery_window_follows_the_nearest_usable_stocked_pickup() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::delivery_window;
    use top_down_2d_rts_prototype_nano_swarm::resources::Stockpile;

    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let rival = app
        .world_mut()
        .spawn((
            top_down_2d_rts_prototype_nano_swarm::nanobot::Swarm {},
            SwarmId(7),
        ))
        .id();
    let mut busy = ProductionFacility::new();
    busy.current_target = Some(NanobotType::Worker);
    let facility = app
        .world_mut()
        .spawn((busy, OwnerSwarm(swarm), Transform::from_xyz(0.0, 0.0, 0.0)))
        .id();
    // Terminals draw from Sources and Sinks alike, so either can be nearest.
    let near = common::spawn_sink_stockpile(&mut app, Vec2::new(600.0, 0.0), 30, 60);
    let far = common::spawn_stockpile(&mut app, Vec2::new(-3_000.0, 0.0), 30, 60);
    for pickup in [near, far] {
        app.world_mut().entity_mut(pickup).insert(OwnerSwarm(swarm));
    }
    // Nearer pickups Haulers cannot draw from: a rival's and a broken one.
    let foreign = common::spawn_sink_stockpile(&mut app, Vec2::new(100.0, 0.0), 30, 60);
    app.world_mut()
        .entity_mut(foreign)
        .insert(OwnerSwarm(rival));
    let broken = common::spawn_sink_stockpile(&mut app, Vec2::new(0.0, 100.0), 30, 60);
    app.world_mut().entity_mut(broken).insert((
        OwnerSwarm(swarm),
        Structure {
            kind: StructureKind::Basic,
            health: SUPPORT_OPERATIONAL_HEALTH_THRESHOLD - 1,
            ticks_since_maintained: 0,
            paint_suspended: false,
        },
    ));
    let tuning = app.world().resource::<Tuning>().clone();
    let window = |app: &App| {
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .queued_space
    };

    run_allocation_period(&mut app);
    assert_eq!(window(&app), delivery_window(&tuning, 600.0));

    app.world_mut().get_mut::<Stockpile>(near).unwrap().amount = 0;
    run_allocation_period(&mut app);
    let far_window = delivery_window(&tuning, 3_000.0);
    assert!(far_window > delivery_window(&tuning, 600.0));
    assert_eq!(
        window(&app),
        far_window,
        "deliveries now come from the far Source"
    );

    app.world_mut().get_mut::<Stockpile>(far).unwrap().amount = 0;
    run_allocation_period(&mut app);
    assert_eq!(window(&app), 0, "no usable stocked pickup");
    let state = app.world().get::<ProductionFacility>(facility).unwrap();
    assert_eq!(
        state.claimable_capacity(&tuning),
        state.input_capacity + tuning.production.cost_per_bot,
        "one cycle is always offered"
    );
}

#[test]
fn delivery_window_keeps_held_claims_until_they_finish() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        HaulerAssignment, LogisticsReservation, delivery_window,
    };
    use top_down_2d_rts_prototype_nano_swarm::resources::Stockpile;

    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let mut busy = ProductionFacility::new();
    busy.current_target = Some(NanobotType::Worker);
    let facility = app
        .world_mut()
        .spawn((busy, OwnerSwarm(swarm), Transform::from_xyz(0.0, 0.0, 0.0)))
        .id();
    let near = common::spawn_sink_stockpile(&mut app, Vec2::new(600.0, 0.0), 0, 60);
    let far = common::spawn_sink_stockpile(&mut app, Vec2::new(-3_000.0, 0.0), 30, 60);
    for sink in [near, far] {
        app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    }
    let tuning = app.world().resource::<Tuning>().clone();
    let window = |app: &App| {
        app.world()
            .get::<ProductionFacility>(facility)
            .unwrap()
            .queued_space
    };
    run_allocation_period(&mut app);
    let far_window = delivery_window(&tuning, 3_000.0);
    assert_eq!(window(&app), far_window);

    // A Hauler holds a claim sized for the far window when the near Sink
    // restocks.
    let input_capacity = app
        .world()
        .get::<ProductionFacility>(facility)
        .unwrap()
        .input_capacity;
    let mut claim = LogisticsReservation::new(far, facility, ResourceKind::Minerals, 0);
    claim.destination_remaining = input_capacity + far_window;
    let hauler = app
        .world_mut()
        .spawn((
            claim,
            HaulerAssignment {
                source: far,
                sink: facility,
                pickup_role: Default::default(),
            },
        ))
        .id();
    app.world_mut().get_mut::<Stockpile>(near).unwrap().amount = 30;
    run_allocation_period(&mut app);
    assert_eq!(
        window(&app),
        far_window,
        "the window never shrinks below claims Haulers already hold"
    );

    app.world_mut().despawn(hauler);
    run_allocation_period(&mut app);
    assert_eq!(
        window(&app),
        delivery_window(&tuning, 600.0),
        "once the claim finishes the window follows the near Sink again"
    );
}

#[test]
fn delivery_window_follows_measured_delivery_time_once_a_hauler_arrives() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Cargo, HaulerAssignment, LogisticsReservation, delivery_window, window_for_lead,
    };
    use top_down_2d_rts_prototype_nano_swarm::resources::{ResourceKind, StockpileRole};

    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    let facility = app
        .world_mut()
        .spawn((
            ProductionFacility::new(),
            OwnerSwarm(swarm),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ))
        .id();
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(100.0, 0.0), 30, 60);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    let tuning = app.world().resource::<Tuning>().clone();
    let keep_busy = |app: &mut App| {
        let mut state = app
            .world_mut()
            .get_mut::<ProductionFacility>(facility)
            .unwrap();
        state.current_target = Some(NanobotType::Worker);
        state.progress = 0;
    };
    // A stand-in Hauler: it holds a claim and cargo but no simulation system
    // moves it, so the test decides when it arrives.
    let mut claim = LogisticsReservation::new(sink, facility, ResourceKind::Minerals, 10);
    claim.source_remaining = 0;
    let hauler = app
        .world_mut()
        .spawn((
            claim,
            HaulerAssignment {
                source: sink,
                sink: facility,
                pickup_role: StockpileRole::Sink,
            },
            Cargo {
                kind: ResourceKind::Minerals,
                amount: 10,
            },
            Transform::from_xyz(5_000.0, 0.0, 0.0),
        ))
        .id();
    let state = |app: &App| {
        let state = app.world().get::<ProductionFacility>(facility).unwrap();
        (state.measured_lead_ticks, state.queued_space)
    };

    for _ in 0..60 {
        keep_busy(&mut app);
        run_allocation_period(&mut app);
    }
    assert_eq!(state(&app).0, None, "still on its way");
    assert_eq!(
        state(&app).1,
        delivery_window(&tuning, 100.0),
        "estimated from the nearest Sink until a delivery is measured"
    );

    // Arrive at the facility's interaction ring.
    let arrival = top_down_2d_rts_prototype_nano_swarm::nanobot::InteractionRegion::structure(
        &Transform::IDENTITY,
    )
    .approach(Vec2::new(5_000.0, 0.0));
    app.world_mut()
        .get_mut::<Transform>(hauler)
        .unwrap()
        .translation = arrival.extend(0.0);
    keep_busy(&mut app);
    run_allocation_period(&mut app);

    let lead = state(&app).0.expect("arrival was timed");
    // 61 calls of 7 fixed ticks each passed between claim and arrival.
    assert!((400.0..=450.0).contains(&lead), "about 427 ticks: {lead}");
    assert_ne!(
        window_for_lead(&tuning, lead),
        delivery_window(&tuning, 100.0)
    );
    assert_eq!(state(&app).1, window_for_lead(&tuning, lead));
}
