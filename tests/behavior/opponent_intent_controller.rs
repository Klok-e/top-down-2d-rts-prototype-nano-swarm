#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        MatchOutcome, OpponentIntentController, OpponentIntentPlugin, OpponentSwarm, Swarm, SwarmId,
    },
};

#[test]
fn opponent_intent_advances_one_defend_cell_toward_the_player() {
    let mut app = common::sim_app();
    app.add_plugins(OpponentIntentPlugin);
    let opponent = SwarmId(11);
    let start = IVec2::new(3, 0);
    let target = IVec2::ZERO;
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(start, IntentKind::Defend, opponent);
    app.world_mut().spawn((
        Swarm {},
        OpponentSwarm {},
        opponent,
        OpponentIntentController::new(start, target, 0, 1),
    ));

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    assert!(!grid.cell(start).unwrap().has(IntentKind::Defend));
    assert_eq!(
        grid.cell(IVec2::new(2, 0))
            .unwrap()
            .owners(IntentKind::Defend)
            .collect::<Vec<_>>(),
        vec![opponent]
    );
}

#[test]
fn opponent_intent_advances_through_enemy_paint_without_erasing_it() {
    let mut app = common::sim_app();
    app.add_plugins(OpponentIntentPlugin);
    let opponent = SwarmId(11);
    let start = IVec2::new(3, 0);
    let hostile_front = IVec2::new(2, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(start, IntentKind::Defend, opponent);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        hostile_front,
        IntentKind::Defend,
        SwarmId::PLAYER,
    );
    app.world_mut().spawn((
        Swarm {},
        OpponentSwarm {},
        opponent,
        OpponentIntentController::new(start, IVec2::ZERO, 0, 1),
    ));

    app.update();
    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .cell(hostile_front)
            .unwrap()
            .owners(IntentKind::Defend)
            .collect::<Vec<_>>(),
        vec![SwarmId::PLAYER, opponent]
    );
    app.update();
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(
        grid.cell(hostile_front)
            .unwrap()
            .owners(IntentKind::Defend)
            .collect::<Vec<_>>(),
        vec![SwarmId::PLAYER]
    );
    assert!(
        grid.cell(IVec2::new(1, 0))
            .unwrap()
            .has_owned(IntentKind::Defend, opponent)
    );
}

#[test]
fn opponent_intent_stops_after_match_outcome_is_latched() {
    let mut app = common::sim_app();
    app.insert_resource(MatchOutcome::Winner(SwarmId::PLAYER));
    app.add_plugins(OpponentIntentPlugin);
    let opponent = SwarmId(11);
    let start = IVec2::new(3, 0);
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .paint(start, IntentKind::Defend, opponent);
    app.world_mut().spawn((
        Swarm {},
        OpponentSwarm {},
        opponent,
        OpponentIntentController::new(start, IVec2::ZERO, 0, 1),
    ));

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(
        grid.cell(start)
            .unwrap()
            .owners(IntentKind::Defend)
            .collect::<Vec<_>>(),
        vec![opponent]
    );
    assert!(!grid.cell(IVec2::new(2, 0)).unwrap().has(IntentKind::Defend));
}

#[test]
fn two_ai_swarms_advance_into_shared_defend_cell() {
    let mut app = common::sim_app();
    app.add_plugins(OpponentIntentPlugin);
    for (id, start, target) in [
        (SwarmId::PLAYER, IVec2::new(1, 1), IVec2::new(4, 4)),
        (SwarmId(1), IVec2::new(3, 3), IVec2::ZERO),
    ] {
        app.world_mut()
            .resource_mut::<IntentGrid>()
            .paint(start, IntentKind::Defend, id);
        app.world_mut().spawn((
            Swarm {},
            id,
            OpponentIntentController::new(start, target, 0, 1),
        ));
    }
    app.update();
    let grid = app.world().resource::<IntentGrid>();
    assert!(
        grid.cell(IVec2::new(2, 2))
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId::PLAYER)
    );
    assert!(
        grid.cell(IVec2::new(2, 2))
            .unwrap()
            .has_owned(IntentKind::Defend, SwarmId(1))
    );
}
