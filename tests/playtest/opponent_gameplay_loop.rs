#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        CollapsePlugin, CombatPlugin, MaintenancePlugin, MatchOutcome, NanobotType,
        OpponentIntentController, OpponentIntentPlugin, PopulationDemandPlugin,
        ProductionCollapseState, ProductionPlugin, ProductionPriority, SwarmId,
        nanobot_death_cleanup_system,
    },
};

#[test]
fn scripted_counter_assault_can_cause_opponent_production_collapse() {
    let mut app = common::sim_app();
    app.insert_resource(ProductionPriority::default());
    app.add_plugins(MaintenancePlugin)
        .add_plugins(ProductionPlugin)
        .add_plugins(PopulationDemandPlugin)
        .add_plugins(CombatPlugin)
        .add_plugins(OpponentIntentPlugin)
        .add_plugins(CollapsePlugin)
        .add_systems(FixedLast, nanobot_death_cleanup_system);

    let player_cell = IVec2::new(-1, 0);
    let opponent_cell = IVec2::ZERO;
    let player_pos = common::cell_world_center(player_cell);
    let opponent_pos = common::cell_world_center(opponent_cell);
    let player_swarm = common::spawn_swarm_with_nanobots(
        &mut app,
        player_pos,
        &[
            (NanobotType::Worker, 2),
            (NanobotType::Hauler, 2),
            (NanobotType::Defender, 3),
        ],
    );
    let opponent_swarm = common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        opponent_pos,
        ProductionPriority::default(),
        &[
            (NanobotType::Worker, 1),
            (NanobotType::Hauler, 1),
            (NanobotType::Defender, 1),
        ],
    );
    let opponent_id = *app.world().entity(opponent_swarm).get::<SwarmId>().unwrap();
    app.world_mut()
        .entity_mut(opponent_swarm)
        .insert(OpponentIntentController::new(
            opponent_cell,
            player_cell,
            0,
            30,
        ));
    {
        let mut grid = app.world_mut().resource_mut::<IntentGrid>();
        grid.paint(player_cell, IntentKind::Defend, SwarmId::PLAYER);
        grid.paint(opponent_cell, IntentKind::Defend, opponent_id);
    }
    common::spawn_facility_at(&mut app, player_swarm, player_pos - Vec2::new(160.0, 0.0));
    common::spawn_facility_at(
        &mut app,
        opponent_swarm,
        opponent_pos + Vec2::new(160.0, 0.0),
    );

    app.update();
    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .cell(player_cell)
            .unwrap()
            .owners(IntentKind::Defend)
            .collect::<Vec<_>>(),
        vec![SwarmId::PLAYER, opponent_id],
        "the opponent controller must launch a advance into independently overlapping paint",
    );

    app.world_mut().resource_mut::<IntentGrid>().paint(
        opponent_cell,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );

    for _ in 0..800 {
        app.update();
        if app
            .world()
            .resource::<ProductionCollapseState>()
            .player_won()
        {
            assert_eq!(
                *app.world().resource::<MatchOutcome>(),
                MatchOutcome::Victory,
                "the visible match result must latch on the collapse tick",
            );
            return;
        }
    }

    let state = app.world().resource::<ProductionCollapseState>();
    panic!(
        "scripted counter-assault did not end the match: player_collapsed={}, opponent_collapsed={}",
        state.player_collapsed, state.opponent_collapsed
    );
}
