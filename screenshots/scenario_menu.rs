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
