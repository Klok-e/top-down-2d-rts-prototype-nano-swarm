use crate::harness::{TestContext, TestFlow};
use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
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
    if ctx.frame <= 6 {
        assert!(ctx.world.resource::<Time<Virtual>>().is_paused());
    }
    if ctx.frame == 5 {
        return TestFlow::Screenshot("scenario_menu".into());
    }
    if ctx.frame == 6 {
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
    if ctx.frame == 7 {
        assert_eq!(
            ctx.world.resource::<ScenarioSelection>().current,
            Scenario::Sandbox
        );
        assert!(!ctx.world.resource::<ScenarioMenu>().open);
        return TestFlow::Screenshot("scenario_started_sandbox".into());
    }
    if ctx.frame < 8 {
        return TestFlow::Continue;
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
