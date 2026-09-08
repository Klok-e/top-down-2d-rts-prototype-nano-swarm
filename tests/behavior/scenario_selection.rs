use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{MatchOutcome, NanobotType, SwarmId},
    scenario_selection::{Scenario, ScenarioSelection},
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn sandbox_remains_open_ended_when_player_is_eliminated() {
    for (scenario, expected) in [
        (Scenario::Standard, MatchOutcome::Winner(SwarmId(1))),
        (Scenario::Sandbox, MatchOutcome::InProgress),
    ] {
        let mut app = common::sim_app_with_elimination();
        let mut selection = ScenarioSelection::default();
        selection.current = scenario;
        app.insert_resource(selection);
        common::spawn_swarm_at(&mut app, Vec2::ZERO);
        common::spawn_opponent_swarm_with_nanobots(
            &mut app,
            Vec2::new(500.0, 0.0),
            &[(NanobotType::Defender, 1)],
        );
        for x in -4..4 {
            app.world_mut().resource_mut::<IntentGrid>().paint(
                IVec2::new(x, 0),
                IntentKind::Defend,
                SwarmId::PLAYER,
            );
        }
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            *app.world().resource::<MatchOutcome>(),
            expected,
            "{scenario:?}"
        );
    }
}
