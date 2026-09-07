use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{MatchOutcome, NanobotType, SwarmId},
    scenario_selection::{Scenario, ScenarioSelection},
};
#[path = "../common/mod.rs"]
mod common;

#[test]
fn sandbox_remains_open_ended_when_production_is_irrecoverable() {
    for (scenario, expected) in [
        (Scenario::Standard, MatchOutcome::Defeat),
        (Scenario::Sandbox, MatchOutcome::InProgress),
    ] {
        let mut app = common::sim_app_with_collapse();
        let mut selection = ScenarioSelection::default();
        selection.current = scenario;
        app.insert_resource(selection);
        common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Defender, 1)]);
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
