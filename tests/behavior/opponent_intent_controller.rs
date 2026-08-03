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
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        start,
        IntentKind::Defend,
        Some(opponent),
    );
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
            .owner(IntentKind::Defend),
        Some(opponent),
    );
}

#[test]
fn opponent_intent_contests_hostile_defend_without_leapfrogging_it() {
    let mut app = common::sim_app();
    app.add_plugins(OpponentIntentPlugin);
    let opponent = SwarmId(11);
    let start = IVec2::new(3, 0);
    let hostile_front = IVec2::new(2, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        start,
        IntentKind::Defend,
        Some(opponent),
    );
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        hostile_front,
        IntentKind::Defend,
        Some(SwarmId::PLAYER),
    );
    app.world_mut().spawn((
        Swarm {},
        OpponentSwarm {},
        opponent,
        OpponentIntentController::new(start, IVec2::ZERO, 0, 1),
    ));

    app.update();
    for _ in 0..3 {
        app.update();
    }

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(
        grid.cell(hostile_front).unwrap().owner(IntentKind::Defend),
        None,
    );
    assert!(
        !grid.cell(IVec2::new(1, 0)).unwrap().has(IntentKind::Defend),
        "the opponent must wait for the contested front to resolve",
    );
}

#[test]
fn opponent_intent_stops_after_match_outcome_is_latched() {
    let mut app = common::sim_app();
    app.insert_resource(MatchOutcome::Victory);
    app.add_plugins(OpponentIntentPlugin);
    let opponent = SwarmId(11);
    let start = IVec2::new(3, 0);
    app.world_mut().resource_mut::<IntentGrid>().paint_owned(
        start,
        IntentKind::Defend,
        Some(opponent),
    );
    app.world_mut().spawn((
        Swarm {},
        OpponentSwarm {},
        opponent,
        OpponentIntentController::new(start, IVec2::ZERO, 0, 1),
    ));

    app.update();

    let grid = app.world().resource::<IntentGrid>();
    assert_eq!(
        grid.cell(start).unwrap().owner(IntentKind::Defend),
        Some(opponent),
    );
    assert!(!grid.cell(IVec2::new(2, 0)).unwrap().has(IntentKind::Defend));
}
