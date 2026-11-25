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
    let source = common::spawn_stockpile(&mut app, Vec2::new(0.0, 300.0), 0, 1_000);
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

/// Spawn a mineral stockpile holding `amount` at `position` for `swarm`.
fn spawn_owned_stock(app: &mut App, swarm: Entity, amount: u32, position: Vec2) {
    app.world_mut().spawn((
        Stockpile {
            kind: ResourceKind::Minerals,
            amount,
            capacity: amount,
            radius: 32.0,
        },
        OwnerSwarm(swarm),
        Transform::from_translation(position.extend(0.0)),
    ));
}

#[test]
fn a_full_reserve_drops_gathering_to_one_per_deposit() {
    let (mut app, swarm, _) = gather_quota_app();
    spawn_owned_stock(&mut app, swarm, 10_000, Vec2::splat(2_000.0));
    app.update();

    let economy = app
        .world()
        .resource::<GatherQuotas>()
        .economy(SwarmId::PLAYER)
        .unwrap();
    assert!(
        economy.reserve_stock > economy.reserve_target,
        "{economy:?}"
    );
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

#[test]
fn haul_quota_sizes_hauler_demand_once_a_window_is_measured() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{
        Cargo, HAUL_WINDOW_TICKS, HaulQuotas, throughput_quota,
    };

    let mut app = common::sim_app_with_production();
    let swarm = common::spawn_swarm_at(&mut app, Vec2::ZERO);
    // Twenty Defend cells keep Defender demand well above what one facility
    // produces during the test, so it never goes idle.
    for x in 0..20 {
        app.world_mut().resource_mut::<IntentGrid>().paint(
            IVec2::new(x, 2),
            IntentKind::Defend,
            SwarmId::PLAYER,
        );
    }
    let facility = common::spawn_facility_at(&mut app, swarm, Vec2::new(0.0, -600.0));
    // Stock far above the Reserve Target, so there is flow to deliver.
    let sink = common::spawn_sink_stockpile(&mut app, Vec2::new(600.0, 0.0), 1_000, 1_000);
    app.world_mut().entity_mut(sink).insert(OwnerSwarm(swarm));
    // Eight busy Haulers share one facility's consumption, so each delivers
    // little and the quota outgrows the few pickup trips one facility offers.
    for i in 0..8 {
        let hauler = common::spawn_hauler_at(&mut app, Vec2::new(600.0 + 20.0 * i as f32, -600.0));
        app.world_mut()
            .entity_mut(hauler)
            .insert(Cargo::empty(ResourceKind::Minerals));
    }
    let economy = |app: &App| {
        app.world()
            .resource::<HaulQuotas>()
            .economy(SwarmId::PLAYER)
    };
    let hauler_demand = |app: &App| {
        app.world()
            .resource::<PopulationDemand>()
            .desired_for(SwarmId::PLAYER, NanobotType::Hauler)
    };

    for _ in 0..HAUL_WINDOW_TICKS / 2 {
        common::fill_facility_input(&mut app, facility);
        app.update();
    }
    assert_eq!(economy(&app), None, "no quota before a full window");

    for _ in 0..HAUL_WINDOW_TICKS {
        common::fill_facility_input(&mut app, facility);
        app.update();
    }
    let economy = economy(&app).expect("a window has finished");
    assert!(economy.delivered_rate > 0.0, "{economy:?}");
    assert!(economy.haul_rate > 0.0, "{economy:?}");
    assert!(economy.target_flow > 0.0, "{economy:?}");
    let growth = app.world().resource::<Tuning>().logistics.haul_growth_step;
    let step = ((economy.haulers as f32 * growth).floor() as u32).max(1);
    let expected =
        throughput_quota(economy.target_flow, economy.haul_rate).min(economy.haulers + step);
    assert!(expected >= 4, "{economy:?}");
    assert_eq!(economy.haul_quota, expected);
    assert_eq!(
        hauler_demand(&app),
        expected,
        "the Haul Quota, not pickup trips, sizes Hauler demand"
    );
}

#[test]
fn far_stock_counts_by_how_soon_it_can_reach_a_consumer() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::reserve_weight;

    let reserve = |offset: f32| {
        let (mut app, swarm, _) = gather_quota_app();
        let mut tuning = app.world().resource::<Tuning>().clone();
        // Loading a full load fits well inside the reserve horizon, so only
        // distance decides.
        tuning.logistics.material_work_ticks = 6;
        app.insert_resource(tuning);
        spawn_owned_stock(&mut app, swarm, 200, Vec2::new(2_000.0 + offset, 2_000.0));
        app.update();
        app.world()
            .resource::<GatherQuotas>()
            .economy(SwarmId::PLAYER)
            .unwrap()
    };
    let near = reserve(100.0);
    let far = reserve(9_000.0);
    assert_eq!(near.reserve_stock, 200, "a short delivery counts in full");
    let tuning = {
        let mut tuning = Tuning::default();
        tuning.logistics.material_work_ticks = 6;
        tuning
    };
    let weight = reserve_weight(&tuning, Some(9_000.0));
    assert!(weight < 0.5, "{weight}");
    assert_eq!(far.reserve_stock, (200.0 * weight).round() as u32);
    assert!(
        far.gather_quota > near.gather_quota,
        "stock far from consumers leaves more gathering to do: {far:?} vs {near:?}"
    );
}

#[test]
fn stock_with_no_consumer_does_not_count() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::reserve_weight;

    assert_eq!(reserve_weight(&Tuning::default(), None), 0.0);
}
