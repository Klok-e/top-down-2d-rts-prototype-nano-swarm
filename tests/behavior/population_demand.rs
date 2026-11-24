use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        GatherAssignment, GatherQuotas, NanobotType, OwnerSwarm, PopulationDemand,
        ProductionFacility, Swarm, SwarmId, SwarmMember, gather_quota, gather_worker_slots,
        spending_capacity,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile},
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn swarm_tiles_create_rounded_reserve() {
    let mut app = common::sim_app_with_population_demand();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(IVec2::ZERO, IntentKind::Gather, SwarmId::PLAYER);
        grid.paint(IVec2::new(1, 0), IntentKind::Build, SwarmId::PLAYER);
        grid.paint(IVec2::new(2, 0), IntentKind::Defend, SwarmId::PLAYER);
    }
    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Worker), 0);
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Hauler), 0);
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        2,
        "three unique Swarm Tiles require a peaceful reserve of two Defenders",
    );
}

#[test]
fn single_owner_paint_creates_no_defender_demand_for_other_swarms() {
    let mut app = common::sim_app_with_population_demand();
    let opponent = SwarmId(7);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );

    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        1
    );
    assert_eq!(demand.desired_for(opponent, NanobotType::Defender), 0);
}

#[test]
fn overlapping_defend_creates_one_swarm_tile_for_each_owner() {
    let mut app = common::sim_app_with_population_demand();
    let opponent = SwarmId(7);
    let cell = IVec2::ZERO;
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Defend, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Defend, opponent);
    }
    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        1,
    );
    assert_eq!(demand.desired_for(opponent, NanobotType::Defender), 1);
}

#[test]
fn erasing_one_owners_overlap_preserves_the_other_owners_demand() {
    let mut app = common::sim_app_with_population_demand();
    let opponent = SwarmId(7);
    let cell = IVec2::ZERO;
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(cell, IntentKind::Defend, SwarmId::PLAYER);
        grid.paint(cell, IntentKind::Defend, opponent);
    }
    app.update();

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .erase(cell, IntentKind::Defend, SwarmId::PLAYER);
    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        0,
    );
    assert_eq!(demand.desired_for(opponent, NanobotType::Defender), 1);
}

#[test]
fn active_threat_count_replaces_peaceful_reserve_when_larger() {
    let mut app = common::sim_app_with_population_demand();
    let opponent = SwarmId(7);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        for x in 0..4 {
            grid.paint(IVec2::new(x, 0), IntentKind::Corridor, SwarmId::PLAYER);
        }
    }
    for x in [64.0, 128.0, 192.0] {
        let hostile = common::spawn_worker_at(&mut app, Vec2::new(x, 64.0));
        app.world_mut()
            .entity_mut(hostile)
            .insert(SwarmMember::new(opponent));
    }

    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        3,
        "three physical Threats exceed the four-tile peaceful reserve of two",
    );
}

#[test]
fn gather_work_creates_worker_demand_not_generic_population() {
    let mut app = common::sim_app_with_population_demand();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, SwarmId(7)));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(IVec2::ZERO, IntentKind::Gather, SwarmId(7));
    app.world_mut().spawn((
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 1_000,
            capacity: 1_000,
            radius: 32.0,
        },
        Transform::from_xyz(32.0, 32.0, 0.0),
    ));

    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    // Neither swarm spends minerals, so each keeps one gatherer on the deposit.
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Worker), 1);
    assert_eq!(demand.desired_for(SwarmId(7), NanobotType::Worker), 1);
    assert_eq!(demand.desired_for(SwarmId(7), NanobotType::Defender), 1);
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Hauler), 0);
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        1,
        "owned Gather intent is also one Swarm Tile",
    );
}

fn gather_quota_app() -> (App, Entity, Entity) {
    let mut app = common::sim_app_with_population_demand();
    let swarm = app.world_mut().spawn((Swarm {}, SwarmId::PLAYER)).id();
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    app.world_mut().spawn((
        ResourceDeposit {
            kind: ResourceKind::Minerals,
            amount: 1_000,
            capacity: 1_000,
            radius: 32.0,
        },
        Transform::from_xyz(32.0, 32.0, 0.0),
    ));
    let mut facility = ProductionFacility::new();
    facility.current_target = Some(NanobotType::Worker);
    app.world_mut().spawn((
        facility,
        OwnerSwarm(swarm),
        Transform::from_xyz(2_000.0, 2_000.0, 0.0),
    ));
    let source = common::spawn_stockpile(&mut app, Vec2::new(0.0, 600.0), 0, 1_000);
    app.world_mut().entity_mut(source).insert(OwnerSwarm(swarm));
    (app, swarm, source)
}

#[test]
fn spending_capacity_raises_the_gather_quota() {
    let (mut app, _, _) = gather_quota_app();
    app.update();

    let tuning = Tuning::default();
    let spending = spending_capacity(1, 0, &tuning);
    let expected = gather_quota(
        spending,
        0,
        tuning.production.reserve_seconds,
        tuning.production.gather_rate_fallback,
        1,
        gather_worker_slots(32.0),
    );
    // 4/s spending plus a 40-mineral reserve gap closed over 10 s needs 8/s;
    // at the 0.3/s fallback that is 27 gatherers, held to the deposit's ring.
    assert_eq!(expected, 6);
    let economy = app
        .world()
        .resource::<GatherQuotas>()
        .economy(SwarmId::PLAYER)
        .expect("live swarm has an economy");
    assert_eq!(economy.gather_quota, expected);
    assert_eq!(
        economy.reserve_target,
        (spending * tuning.production.reserve_seconds).round() as u32
    );
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        expected
    );
}

#[test]
fn a_full_reserve_drops_gathering_to_one_per_deposit() {
    let (mut app, swarm, _) = gather_quota_app();
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount: 10_000,
            capacity: 10_000,
            radius: 32.0,
        },
        OwnerSwarm(swarm),
        Transform::from_xyz(-2_000.0, -2_000.0, 0.0),
    ));
    app.update();

    let economy = app
        .world()
        .resource::<GatherQuotas>()
        .economy(SwarmId::PLAYER)
        .unwrap();
    assert_eq!(economy.reserve_stock, 10_000);
    assert_eq!(economy.gather_quota, 1);
    assert_eq!(
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Worker),
        1
    );
}

#[test]
fn idle_workers_beyond_the_gather_quota_do_not_gather() {
    let mut app = common::sim_app_with_gather();
    app.add_plugins(top_down_2d_rts_prototype_nano_swarm::nanobot::PopulationDemandPlugin);
    let center = common::cell_world_center(IVec2::ZERO);
    let swarm = common::spawn_swarm_at(&mut app, center);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: center,
            amount: 100_000,
            capacity: 100_000,
            radius: 64.0,
        },
    );
    let stockpile = common::spawn_stockpile(&mut app, center + Vec2::new(0.0, 300.0), 0, 1_000);
    app.world_mut()
        .entity_mut(stockpile)
        .insert(OwnerSwarm(swarm));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    let workers: Vec<_> = (0..5)
        .map(|i| common::spawn_worker_at(&mut app, center + Vec2::new(-200.0, 40.0 * i as f32)))
        .collect();
    let gathering = |app: &App| {
        workers
            .iter()
            .filter(|worker| app.world().get::<GatherAssignment>(**worker).is_some())
            .count()
    };
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(
        gathering(&app),
        1,
        "with nothing to spend on, one Worker keeps the deposit worked"
    );

    let mut facility = ProductionFacility::new();
    facility.current_target = Some(NanobotType::Worker);
    app.world_mut().spawn((
        facility,
        OwnerSwarm(swarm),
        Transform::from_translation((center + Vec2::new(2_000.0, 0.0)).extend(0.0)),
    ));
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(
        gathering(&app),
        5,
        "a producing facility raises the quota and idle Workers start gathering"
    );
}

#[test]
fn gatherers_are_capped_by_full_loads_of_source_room() {
    let (mut app, _, source) = gather_quota_app();
    let carry = Tuning::default().logistics.worker_carry_capacity;
    {
        let mut stockpile = app.world_mut().get_mut::<Stockpile>(source).unwrap();
        stockpile.capacity = carry;
    }
    app.update();

    let economy = app
        .world()
        .resource::<GatherQuotas>()
        .economy(SwarmId::PLAYER)
        .unwrap();
    assert_eq!(
        economy.gather_quota, 2,
        "one gatherer per deposit plus one full load of Source room"
    );
}

#[test]
fn a_deposit_takes_no_more_gatherers_than_fit_on_its_ring() {
    let mut app = common::sim_app_with_gather();
    let center = common::cell_world_center(IVec2::ZERO);
    let swarm = common::spawn_swarm_at(&mut app, center);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: center,
            amount: 100_000,
            capacity: 100_000,
            radius: 24.0,
        },
    );
    let stockpile = common::spawn_stockpile(&mut app, center + Vec2::new(0.0, 300.0), 0, 1_000);
    app.world_mut()
        .entity_mut(stockpile)
        .insert(OwnerSwarm(swarm));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    let workers: Vec<_> = (0..8)
        .map(|i| common::spawn_worker_at(&mut app, center + Vec2::new(-200.0, 30.0 * i as f32)))
        .collect();
    for _ in 0..30 {
        app.update();
    }

    let gathering = workers
        .iter()
        .filter(|worker| app.world().get::<GatherAssignment>(**worker).is_some())
        .count();
    assert_eq!(gathering as u32, gather_worker_slots(24.0));
    assert!(gathering < workers.len());
}

#[test]
fn workers_beyond_the_gather_quota_take_construction_or_idle() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        OpportunityCategory, PopulationDemandPlugin, RegionalLease,
    };
    let mut app = common::sim_app_with_gather_planned();
    app.add_plugins(PopulationDemandPlugin);
    let center = common::cell_world_center(IVec2::ZERO);
    let swarm = common::spawn_swarm_at(&mut app, center);
    common::spawn_deposit(
        &mut app,
        common::DepositFixture {
            world_pos: center,
            amount: 100_000,
            capacity: 100_000,
            radius: 64.0,
        },
    );
    let stockpile = common::spawn_stockpile(&mut app, center + Vec2::new(0.0, 300.0), 0, 1_000);
    app.world_mut()
        .entity_mut(stockpile)
        .insert(OwnerSwarm(swarm));
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::ZERO,
        IntentKind::Gather,
        SwarmId::PLAYER,
    );
    let build_cell = IVec2::new(1, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        build_cell,
        IntentKind::Build,
        SwarmId::PLAYER,
    );
    common::spawn_planned_structure_at_cell(&mut app, build_cell);
    let workers: Vec<_> = (0..3)
        .map(|i| common::spawn_worker_at(&mut app, center + Vec2::new(-200.0, 40.0 * i as f32)))
        .collect();
    let leases = |app: &App| {
        workers
            .iter()
            .filter_map(|worker| app.world().get::<RegionalLease>(*worker))
            .map(|lease| lease.category)
            .collect::<Vec<_>>()
    };
    for _ in 0..300 {
        app.update();
        if leases(&app).len() == 2 {
            break;
        }
    }
    for _ in 0..30 {
        app.update();
    }

    // With nothing to spend on, the quota is one gatherer: the third Worker
    // idles instead of joining the deposit.
    let categories = leases(&app);
    let count = |category| categories.iter().filter(|c| **c == category).count();
    assert_eq!(count(OpportunityCategory::Gather), 1, "{categories:?}");
    assert_eq!(
        count(OpportunityCategory::PlannedBuild),
        1,
        "{categories:?}"
    );
    assert_eq!(categories.len(), 2, "{categories:?}");
}
