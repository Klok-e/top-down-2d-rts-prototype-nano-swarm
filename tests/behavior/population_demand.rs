use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{NanobotType, PopulationDemand, Swarm, SwarmId, SwarmMember},
    resources::{ResourceDeposit, ResourceKind},
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
