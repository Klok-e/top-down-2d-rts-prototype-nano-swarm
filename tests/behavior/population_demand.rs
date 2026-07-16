use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActionableProjection, DefendPressure, NanobotType, PopulationDemand, Swarm, SwarmId,
        population_demand_system, project_actionable_opportunities_system,
    },
    resources::{ResourceDeposit, ResourceKind},
};

fn demand_app() -> App {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(32, 32))
        .init_resource::<ActionableProjection>()
        .init_resource::<DefendPressure>()
        .init_resource::<PopulationDemand>()
        .add_systems(
            Update,
            (
                project_actionable_opportunities_system,
                population_demand_system,
            )
                .chain(),
        );
    app
}

#[test]
fn defend_cells_and_hostiles_create_typed_defender_demand() {
    let mut app = demand_app();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint_owned(IVec2::ZERO, IntentKind::Defend, Some(SwarmId::PLAYER));
        grid.paint_owned(IVec2::new(1, 0), IntentKind::Defend, Some(SwarmId::PLAYER));
    }
    app.world_mut()
        .resource_mut::<DefendPressure>()
        .set(IVec2::ZERO, 3.0);

    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Worker), 0);
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Hauler), 0);
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        4,
        "two cells contribute two baseline Defenders and pressure adds two more",
    );
}

#[test]
fn unowned_defend_work_creates_demand_for_every_visible_swarm() {
    let mut app = demand_app();
    let opponent = SwarmId(7);
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().spawn((Swarm {}, opponent));
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(IVec2::ZERO, IntentKind::Defend);

    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        1
    );
    assert_eq!(demand.desired_for(opponent, NanobotType::Defender), 1);
}

#[test]
fn gather_work_creates_worker_demand_not_generic_population() {
    let mut app = demand_app();
    app.world_mut().spawn((Swarm {}, SwarmId::PLAYER));
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        IVec2::ZERO,
        IntentKind::Gather,
        Some(SwarmId::PLAYER),
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

    app.update();

    let demand = app.world().resource::<PopulationDemand>();
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Worker), 1);
    assert_eq!(demand.desired_for(SwarmId::PLAYER, NanobotType::Hauler), 0);
    assert_eq!(
        demand.desired_for(SwarmId::PLAYER, NanobotType::Defender),
        0
    );
}
