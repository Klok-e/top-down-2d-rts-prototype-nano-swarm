use crate::harness::{TestContext, TestFlow};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::MatchOutcome,
    scenario_selection::{Scenario, ScenarioSelection},
    ui::scenario_menu::{MenuAction, MenuInputSet, ScenarioMenu},
};

pub fn scenario_menu(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 2 {
        ctx.world.resource_mut::<ScenarioMenu>().open = true;
        ctx.world.resource_mut::<ScenarioSelection>().next_launch = Scenario::Sandbox;
    }
    if ctx.frame < 5 {
        return TestFlow::Continue;
    }
    if ctx.frame <= 8 {
        assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
    }
    if ctx.frame == 5 {
        return TestFlow::Screenshot("scenario_menu".into());
    }
    if ctx.frame == 6 {
        ctx.world.insert_resource(MatchOutcome::Draw);
        return TestFlow::Continue;
    }
    if ctx.frame == 7 {
        return TestFlow::Screenshot("scenario_menu_match_complete".into());
    }
    if ctx.frame == 8 {
        let start = ctx
            .world
            .query::<(Entity, &MenuAction)>()
            .iter(ctx.world)
            .find_map(|(entity, action)| (*action == MenuAction::Start).then_some(entity))
            .expect("start button exists");
        // Inject after UI focus, at the same boundary used by agent-control input.
        ctx.world
            .resource_mut::<Schedules>()
            .get_mut(PreUpdate)
            .unwrap()
            .add_systems(
                (move |mut buttons: Query<&mut Interaction>, mut pressed: Local<bool>| {
                    if !*pressed {
                        *buttons.get_mut(start).unwrap() = Interaction::Pressed;
                        *pressed = true;
                    }
                })
                .after(MenuInputSet::Keyboard)
                .before(MenuInputSet::Actions),
            );
        return TestFlow::Continue;
    }
    if ctx.frame == 9 {
        assert_eq!(
            ctx.world.resource::<ScenarioSelection>().current,
            Scenario::Sandbox
        );
        assert!(!ctx.world.resource::<ScenarioMenu>().open);
        return TestFlow::Screenshot("scenario_started_sandbox".into());
    }
    if ctx.frame < 10 {
        return TestFlow::Continue;
    }
    TestFlow::Exit
}

pub fn ai_battle_spectator(ctx: &mut TestContext) -> TestFlow {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{StrategicController, SwarmId};
    if ctx.frame == 2 {
        assert_eq!(
            ctx.world.resource::<ScenarioSelection>().current,
            Scenario::AiBattle
        );
        assert_eq!(
            ctx.world
                .query::<(&SwarmId, &StrategicController)>()
                .iter(ctx.world)
                .count(),
            2
        );
        return TestFlow::Screenshot("ai_battle_spectator".into());
    }
    if ctx.frame > 2 {
        return TestFlow::Exit;
    }
    TestFlow::Continue
}
