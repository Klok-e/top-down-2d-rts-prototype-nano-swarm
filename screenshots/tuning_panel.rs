use bevy::{prelude::*, ui::UiGlobalTransform};
use top_down_2d_rts_prototype_nano_swarm::{
    intent::IntentKind,
    tuning::Tuning,
    ui::{
        intent_layer_panel::IntentLayerButton,
        tuning_panel::{TuningEditor, TuningPanelRoot, edit_value},
    },
};

use crate::harness::{TestContext, TestFlow};

pub fn tuning_panel(ctx: &mut TestContext) -> TestFlow {
    if ctx.frame == 0 {
        ctx.world.resource_mut::<TuningEditor>().set_open(true);
    }
    if ctx.frame == 2 {
        let panel = ctx
            .world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<TuningPanelRoot>>()
            .single(ctx.world)
            .unwrap();
        let panel = Rect::from_center_size(panel.1.translation, panel.0.size());
        let mut buttons = ctx
            .world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<IntentLayerButton>>();
        assert_eq!(buttons.iter(ctx.world).count(), IntentKind::COUNT);
        for (button, transform) in buttons.iter(ctx.world) {
            let button = Rect::from_center_size(transform.translation, button.size());
            let overlap = panel.intersect(button);
            assert!(
                overlap.is_empty(),
                "the tuning panel covers a brush button at 800×600: {overlap:?}"
            );
        }
        assert!(
            panel.max.y < 600.0,
            "the collapsed panel must end before the bottom of the viewport"
        );
        return TestFlow::Screenshot("tuning_panel_collapsed".into());
    }
    if ctx.frame == 3 {
        let mut editor = ctx.world.resource_mut::<TuningEditor>();
        for group in ["production", "construction", "logistics"] {
            editor.set_group_expanded(group, true);
        }
    }
    if ctx.frame == 5 {
        assert!(ctx.world.resource::<TuningEditor>().open);
        assert!(!ctx.world.resource::<Time<Virtual>>().is_paused());
        return TestFlow::Screenshot("tuning_panel_expanded".into());
    }
    if ctx.frame == 6 {
        let before = ctx.world.resource::<Tuning>().clone();
        assert!(edit_value(ctx.world, "production.cost_per_bot", "1000001").is_err());
        assert_eq!(ctx.world.resource::<Tuning>(), &before);
    }
    if ctx.frame == 7 {
        assert!(
            ctx.world
                .resource::<TuningEditor>()
                .errors
                .contains_key("production.cost_per_bot")
        );
        return TestFlow::Screenshot("tuning_panel_error".into());
    }
    if ctx.frame == 8 {
        let mut editor = ctx.world.resource_mut::<TuningEditor>();
        editor.errors.clear();
        for group in ["production", "construction", "logistics"] {
            editor.set_group_expanded(group, false);
        }
        editor.set_group_expanded("presentation", true);
    }
    if ctx.frame == 10 {
        return TestFlow::Screenshot("tuning_panel_presentation".into());
    }
    if ctx.frame == 11 {
        let mut panels = ctx
            .world
            .query_filtered::<(&ComputedNode, &mut ScrollPosition), With<TuningPanelRoot>>();
        for (node, mut position) in panels.iter_mut(ctx.world) {
            let overflow =
                (node.content_size().y - node.size().y).max(0.0) * node.inverse_scale_factor();
            assert!(
                overflow > 0.0,
                "expanded presentation settings must be scrollable"
            );
            position.0.y = overflow;
        }
    }
    if ctx.frame == 13 {
        return TestFlow::Screenshot("tuning_panel_scrolled".into());
    }
    if ctx.frame > 13 {
        TestFlow::Exit
    } else {
        TestFlow::Continue
    }
}
