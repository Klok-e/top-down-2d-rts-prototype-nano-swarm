use crate::harness::{TestContext, TestFlow};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    scenario_selection::{Scenario, ScenarioSelection},
    ui::scenario_menu::ScenarioMenu,
};

pub fn scenario_menu(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        ctx.world.resource_mut::<ScenarioMenu>().open = true;
        ctx.world.resource_mut::<ScenarioSelection>().next_launch = Scenario::Sandbox;
    }
    if ctx.frame < 5 {
        return TestFlow::Continue;
    }
    assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
    if ctx.frame == 5 {
        return TestFlow::Screenshot("scenario_menu".into());
    }
    TestFlow::Exit
}

pub fn ai_battle_spectator(ctx: &mut TestContext) -> TestFlow {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{OpponentIntentController, SwarmId};
    if ctx.frame == 2 {
        assert_eq!(
            ctx.world.resource::<ScenarioSelection>().current,
            Scenario::AiBattle
        );
        assert_eq!(
            ctx.world
                .query::<(&SwarmId, &OpponentIntentController)>()
                .iter(ctx.world)
                .count(),
            2
        );
        assert!(
            ctx.world
                .query::<&Text>()
                .iter(ctx.world)
                .any(|text| text.0.contains("Spectating"))
        );
        return TestFlow::Screenshot("ai_battle_spectator".into());
    }
    if ctx.frame > 2 {
        return TestFlow::Exit;
    }
    TestFlow::Continue
}
