//! Behavior tests for the right-side Production Ratio panel
//! (issue #32). Drive the Bevy systems end-to-end so the
//! panel wiring (clicks -> resource mutation, panel
//! membership filter, player-only effect, opponent isolation,
//! value-text refresh) is pinned by a runnable test. The
//! pure math lives in `src/ui/production_ratio_panel.rs` and
//! `src/nanobot/production.rs`.

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{spawn_opponent_swarm, NanobotType, ProductionRatio, SwarmProduction},
    ui::{
        production_ratio_panel::{
            production_ratio_slider_click_system, setup_production_ratio_panel,
            update_production_ratio_value_texts, ProductionRatioPanelRoot, ProductionRatioSlider,
            ProductionRatioValueText, SliderDirection, SLIDER_STEP,
        },
        FontsResource,
    },
};

#[path = "../common/mod.rs"]
mod common;

use common::press_button;

fn build_app() -> App {
    // Minimal Bevy `App` with the two panel systems and a
    // default `ProductionRatio`. The setup system reads
    // `FontsResource`; we seed a placeholder handle so the
    // panel spawns without the full `DefaultPlugins` stack.
    let mut app = common::minimal_app();
    app.insert_resource(FontsResource {
        font: Handle::default(),
    });
    app.insert_resource(ProductionRatio::default());
    app.add_systems(Startup, setup_production_ratio_panel);
    app.add_systems(Update, production_ratio_slider_click_system);
    app.add_systems(Update, update_production_ratio_value_texts);
    app.update();
    app
}

fn find_slider(world: &mut World, kind: NanobotType, direction: SliderDirection) -> Option<Entity> {
    let mut q = world.query::<(Entity, &ProductionRatioSlider)>();
    q.iter(world)
        .find(|(_, slider)| slider.kind == kind && slider.direction == direction)
        .map(|(e, _)| e)
}

#[test]
fn panel_spawns_one_root_after_setup() {
    // Acceptance: panel is visible during gameplay -> exactly
    // one `ProductionRatioPanelRoot` exists after setup.
    let mut app = build_app();
    let roots: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<ProductionRatioPanelRoot>>()
        .iter(app.world())
        .collect();
    assert_eq!(roots.len(), 1);
}

#[test]
fn panel_has_one_slider_pair_per_type() {
    // 3 types x 2 buttons = 6 sliders.
    let mut app = build_app();
    let sliders: Vec<_> = app
        .world_mut()
        .query::<&ProductionRatioSlider>()
        .iter(app.world())
        .collect();
    assert_eq!(sliders.len(), 6);
    for kind in NanobotType::ALL {
        assert!(sliders
            .iter()
            .any(|s| s.kind == kind && s.direction == SliderDirection::Increase));
        assert!(sliders
            .iter()
            .any(|s| s.kind == kind && s.direction == SliderDirection::Decrease));
    }
}

#[test]
fn panel_has_value_text_per_type() {
    let mut app = build_app();
    let texts: Vec<_> = app
        .world_mut()
        .query::<&ProductionRatioValueText>()
        .iter(app.world())
        .collect();
    let kinds: std::collections::HashSet<_> = texts.iter().map(|t| t.kind).collect();
    assert_eq!(texts.len(), 3);
    for kind in NanobotType::ALL {
        assert!(kinds.contains(&kind));
    }
}

#[test]
fn click_increase_button_raises_weight_by_step() {
    let mut app = build_app();
    let before = app
        .world()
        .resource::<ProductionRatio>()
        .weight(NanobotType::Worker);
    let button = find_slider(
        app.world_mut(),
        NanobotType::Worker,
        SliderDirection::Increase,
    )
    .expect("Worker + button must exist");
    press_button(&mut app, button);
    let after = app
        .world()
        .resource::<ProductionRatio>()
        .weight(NanobotType::Worker);
    assert_eq!(after, before + SLIDER_STEP as u32);
}

#[test]
fn click_decrease_button_lowers_weight_by_step() {
    let mut app = build_app();
    let before = app
        .world()
        .resource::<ProductionRatio>()
        .weight(NanobotType::Worker);
    let button = find_slider(
        app.world_mut(),
        NanobotType::Worker,
        SliderDirection::Decrease,
    )
    .expect("Worker - button must exist");
    press_button(&mut app, button);
    let after = app
        .world()
        .resource::<ProductionRatio>()
        .weight(NanobotType::Worker);
    assert_eq!(after, before.saturating_sub(SLIDER_STEP as u32));
}

#[test]
fn click_decrease_rejected_when_only_nonzero_type_remaining() {
    // Acceptance: the last nonzero type is clamped. With only
    // Defender set, a `-` click must not zero the total.
    let mut app = build_app();
    {
        let mut ratio = app.world_mut().resource_mut::<ProductionRatio>();
        ratio.weights.clear();
        ratio.set_weight(NanobotType::Defender, 1);
    }
    let button = find_slider(
        app.world_mut(),
        NanobotType::Defender,
        SliderDirection::Decrease,
    )
    .expect("Defender - button must exist");
    press_button(&mut app, button);
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Defender),
        1
    );
}

#[test]
fn slider_does_not_mutate_opponent_swarm_production() {
    // Acceptance: opponent swarm production keeps its fixed
    // authored ratio even when the player mutates the global
    // ratio via the slider.
    let mut app = build_app();
    let mut opponent_ratio = ProductionRatio::new();
    opponent_ratio.set_weight(NanobotType::Worker, 8);
    opponent_ratio.set_weight(NanobotType::Hauler, 4);
    opponent_ratio.set_weight(NanobotType::Defender, 3);
    let opponent = spawn_opponent_swarm(
        app.world_mut(),
        bevy::math::Vec2::new(2000.0, 0.0),
        opponent_ratio.clone(),
        &[],
        &[],
    );
    let before = app
        .world()
        .entity(opponent)
        .get::<SwarmProduction>()
        .expect("opponent carries SwarmProduction")
        .ratio
        .clone();

    let button = find_slider(
        app.world_mut(),
        NanobotType::Worker,
        SliderDirection::Increase,
    )
    .expect("Worker + button must exist");
    press_button(&mut app, button);
    press_button(&mut app, button);

    let after = app
        .world()
        .entity(opponent)
        .get::<SwarmProduction>()
        .expect("opponent still carries SwarmProduction")
        .ratio
        .clone();
    assert_eq!(before.weights, after.weights);
    // And the player's global ratio moved.
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        6 + 2 * SLIDER_STEP as u32
    );
}

#[test]
fn loose_button_outside_panel_does_not_mutate_ratio() {
    // The panel-membership filter guards against stray
    // buttons driving the global ratio.
    let mut app = build_app();
    let before = app
        .world()
        .resource::<ProductionRatio>()
        .weight(NanobotType::Worker);
    let loose = app
        .world_mut()
        .spawn((
            Button,
            ProductionRatioSlider {
                kind: NanobotType::Worker,
                direction: SliderDirection::Increase,
            },
            Interaction::Pressed,
        ))
        .id();
    app.update();
    app.update();
    let after = app
        .world()
        .resource::<ProductionRatio>()
        .weight(NanobotType::Worker);
    assert_eq!(before, after);
    app.world_mut().entity_mut(loose).despawn();
}

#[test]
fn value_text_reflects_ratio_change() {
    // Default 6/3/1 -> Defender 10%. After a `-` click the
    // 6/3/0 mix renders Defender as 0%.
    let mut app = build_app();
    let defender_text = app
        .world_mut()
        .query_filtered::<Entity, With<ProductionRatioValueText>>()
        .iter(app.world())
        .find(|e| {
            app.world()
                .entity(*e)
                .get::<ProductionRatioValueText>()
                .unwrap()
                .kind
                == NanobotType::Defender
        })
        .expect("Defender value text must exist");
    let text = app
        .world()
        .entity(defender_text)
        .get::<Text>()
        .expect("value text carries Text");
    assert!(text.0.contains("10%"), "got {:?}", text.0);

    let dec = find_slider(
        app.world_mut(),
        NanobotType::Defender,
        SliderDirection::Decrease,
    )
    .expect("Defender - button must exist");
    press_button(&mut app, dec);
    let text = app
        .world()
        .entity(defender_text)
        .get::<Text>()
        .expect("value text still carries Text");
    assert!(text.0.contains("0%"), "got {:?}", text.0);
}
