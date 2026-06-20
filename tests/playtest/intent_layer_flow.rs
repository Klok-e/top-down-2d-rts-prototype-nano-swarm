use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::intent::{
    brush_key_for_kind, brush_selection_keyboard_system, BrushSelection, IntentKind,
};

fn build_app() -> App {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<BrushSelection>()
        .add_systems(Update, brush_selection_keyboard_system);
    app
}

fn press_key(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(key);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.update();
}

#[test]
fn scripted_player_can_select_each_intent_layer_from_keyboard() {
    let mut app = build_app();

    for kind in [
        IntentKind::Build,
        IntentKind::Defend,
        IntentKind::Corridor,
        IntentKind::Gather,
    ] {
        press_key(
            &mut app,
            brush_key_for_kind(kind).expect("playtest layer must have a key binding"),
        );
        assert_eq!(app.world().resource::<BrushSelection>().kind, kind);
    }
}
