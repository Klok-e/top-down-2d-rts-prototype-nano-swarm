//! Integration test for the UI-capture half of the zone-brush fix:
//! the brush must see fresh `is_pointer_over_ui` state on the same
//! frame, the intent-layer panel's full-width layout anchor must
//! not block the brush on its own, and the `check_ui_interaction`
//! system must skip entities tagged with `NoPointerCapture` while
//! still picking up unmarked UI nodes. The tests wire a minimal
//! Bevy `App` (window, camera, cursor, capture + brush systems in
//! the production order) and assert the brush paints or skips
//! based on the current frame's pointer state.

use bevy::camera::RenderTargetInfo;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::Window;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{BrushSelection, IntentGrid},
    ui::{
        check_ui_interaction, intent_layer_panel::IntentLayerPanelRoot, NoPointerCapture,
        UiHandling,
    },
    zones::zone_brush_system,
};

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.insert_resource(UiHandling::default());
    app.insert_resource(BrushSelection::default());
    app.insert_resource(IntentGrid::new(16, 16));
    app.insert_resource(ButtonInput::<MouseButton>::default());
    app
}

fn spawn_window_with_cursor(app: &mut App, cursor: Vec2) -> Entity {
    let entity = app
        .world_mut()
        .spawn(Window {
            resolution: (1280, 720).into(),
            ..default()
        })
        .id();
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Window>()
        .expect("window entity must have a Window component")
        .set_cursor_position(Some(cursor));
    entity
}

fn spawn_camera_at_origin(app: &mut App) -> Entity {
    let entity = app
        .world_mut()
        .spawn((Camera2d, Transform::IDENTITY, GlobalTransform::default()))
        .id();
    // In a headless test the render graph does not run, so the
    // camera's `computed.target_info` is never populated and
    // `Camera::viewport_to_world_2d` returns an error. Pre-fill it
    // with the same physical size the window advertises so the
    // brush's world-coordinate math has something to project to.
    let mut entity_ref = app.world_mut().entity_mut(entity);
    let mut camera = entity_ref
        .get_mut::<Camera>()
        .expect("Camera2d spawn must include a Camera component");
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1280, 720),
        scale_factor: 1.0,
    });
    entity
}

fn press_left_mouse(app: &mut App) {
    let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    input.press(MouseButton::Left);
}

/// Register the capture system before the brush system, matching the
/// production ordering in `NanoswarmUiSetupPlugin`. Every test in
/// this file needs the same chain.
fn add_brush_chain(app: &mut App) {
    app.add_systems(Update, (check_ui_interaction, zone_brush_system).chain());
}

#[test]
fn brush_paints_when_no_ui_capture_is_set_in_current_frame() {
    // The cursor is over the world (well outside the intent panel
    // strip), no UI node says "cursor over", and the player is
    // holding left mouse. The brush must paint at the world cell
    // under the cursor.
    let mut app = build_app();
    spawn_window_with_cursor(&mut app, Vec2::new(640.0, 360.0));
    spawn_camera_at_origin(&mut app);
    add_brush_chain(&mut app);

    press_left_mouse(&mut app);
    app.update();

    // `dirty_count` is the brush's "did anything happen" flag:
    // painting a cell marks it dirty, so a non-zero count means
    // the brush executed its paint path end-to-end.
    let grid = app.world().resource::<IntentGrid>();
    let dirty_count = grid.dirty_count();
    assert!(
        dirty_count > 0,
        "brush must paint at least one cell when the cursor is over the world (dirty cells: {dirty_count})"
    );
}

#[test]
fn brush_skips_paint_when_ui_capture_is_set_in_current_frame() {
    // A UI node (the panel root) says "cursor over" and is tagged
    // without `NoPointerCapture` to simulate an actual button
    // capture. The capture system runs first and sets
    // `is_pointer_over_ui = true`; the brush must then skip the
    // paint.
    let mut app = build_app();
    spawn_window_with_cursor(&mut app, Vec2::new(640.0, 360.0));
    spawn_camera_at_origin(&mut app);
    add_brush_chain(&mut app);

    // Button-like entity: no NoPointerCapture, so check_ui_interaction
    // must pick it up.
    app.world_mut().spawn((
        Node::default(),
        RelativeCursorPosition {
            cursor_over: true,
            normalized: None,
        },
    ));

    press_left_mouse(&mut app);
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let dirty_count = grid.dirty_count();
    assert_eq!(
        dirty_count, 0,
        "brush must not paint when is_pointer_over_ui is true (dirty cells: {dirty_count})"
    );
}

#[test]
fn brush_paints_when_only_no_pointer_capture_root_is_over() {
    // The intent-layer panel root spans the full window width and
    // would otherwise be flagged as "cursor over" for any cursor
    // position. With `NoPointerCapture` the capture system must
    // ignore it, so the brush paints as normal. This is the actual
    // player scenario: the cursor is over the world, but the panel
    // root's broad footprint is still "under" the cursor.
    let mut app = build_app();
    spawn_window_with_cursor(&mut app, Vec2::new(640.0, 360.0));
    spawn_camera_at_origin(&mut app);
    add_brush_chain(&mut app);

    app.world_mut().spawn((
        Node::default(),
        RelativeCursorPosition {
            cursor_over: true,
            normalized: None,
        },
        IntentLayerPanelRoot,
        NoPointerCapture,
    ));

    press_left_mouse(&mut app);
    app.update();

    let grid = app.world().resource::<IntentGrid>();
    let dirty_count = grid.dirty_count();
    assert!(
        dirty_count > 0,
        "brush must paint when only NoPointerCapture nodes are over the cursor (dirty cells: {dirty_count})"
    );
}

#[test]
fn brush_paints_after_check_ui_clears_stale_over_ui_state() {
    // Direct verification of the system-order fix: the
    // `is_pointer_over_ui` resource starts the frame as `true`
    // (stale from a previous frame), no UI node currently has the
    // cursor over it, and the player is holding left mouse. In a
    // single `app.update()` the capture system must clear the
    // stale `true` to `false` before the brush reads it, so the
    // brush paints. If the order were reversed, the brush would
    // see the stale `true` and skip the paint even though no UI
    // is actually over the cursor.
    let mut app = build_app();
    app.world_mut()
        .resource_mut::<UiHandling>()
        .is_pointer_over_ui = true;

    spawn_window_with_cursor(&mut app, Vec2::new(640.0, 360.0));
    spawn_camera_at_origin(&mut app);
    add_brush_chain(&mut app);

    press_left_mouse(&mut app);
    app.update();

    let handling = app.world().resource::<UiHandling>();
    assert!(
        !handling.is_pointer_over_ui,
        "check_ui_interaction must clear the stale over-ui state when no UI is over the cursor"
    );

    let grid = app.world().resource::<IntentGrid>();
    let dirty_count = grid.dirty_count();
    assert!(
        dirty_count > 0,
        "brush must paint after check_ui_interaction clears the stale state \
         (dirty cells: {dirty_count}, order is check_ui then zone_brush)"
    );
}

#[test]
fn check_ui_interaction_ignores_no_pointer_capture_but_picks_up_other_nodes() {
    // The intent layer panel is a full-width layout anchor
    // (`left: 0`, `right: 0`). Without `NoPointerCapture` its
    // `RelativeCursorPosition` would mark the cursor as "over UI"
    // for the whole viewport, blocking the world brush. The
    // `check_ui_interaction` system filters out entities with the
    // marker, so the root's broad footprint is ignored while the
    // descendant buttons still capture pointer state. The control
    // entity (no marker) is included so we can confirm
    // `check_ui_interaction` still flips the resource for real
    // interactive nodes.
    let mut app = App::new();
    app.insert_resource(UiHandling::default());
    app.add_systems(Update, check_ui_interaction);

    // Mimic the panel root: a Node with RelativeCursorPosition that
    // says the cursor is over, and the NoPointerCapture marker the
    // setup system now adds.
    app.world_mut().spawn((
        Node::default(),
        RelativeCursorPosition {
            cursor_over: true,
            normalized: None,
        },
        IntentLayerPanelRoot,
        NoPointerCapture,
    ));

    // Control: a real button-like UI node (no marker) that should
    // still be picked up by `check_ui_interaction`.
    app.world_mut().spawn((
        Node::default(),
        RelativeCursorPosition {
            cursor_over: true,
            normalized: None,
        },
    ));

    app.update();

    let handling = app.world().resource::<UiHandling>();
    assert!(
        handling.is_pointer_over_ui,
        "the descendant button (no marker) must still flip the resource"
    );
}

#[test]
fn check_ui_interaction_keeps_resource_false_when_only_marker_nodes_are_over() {
    // When the panel root is the only thing with cursor-over, the
    // marker must keep `is_pointer_over_ui` false. This is the
    // actual player scenario: cursor over the world, panel root
    // still considers the cursor "near" because of its full-width
    // footprint, so the brush must still paint.
    let mut app = App::new();
    app.insert_resource(UiHandling::default());
    app.add_systems(Update, check_ui_interaction);

    app.world_mut().spawn((
        Node::default(),
        RelativeCursorPosition {
            cursor_over: true,
            normalized: None,
        },
        IntentLayerPanelRoot,
        NoPointerCapture,
    ));

    app.update();

    let handling = app.world().resource::<UiHandling>();
    assert!(
        !handling.is_pointer_over_ui,
        "NoPointerCapture on the panel root must let the brush through"
    );
}
