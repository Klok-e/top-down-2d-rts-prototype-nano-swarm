//! Integration coverage for the zone-painting fix: the background and
//! zone overlays must use `translation.z` for draw order (not
//! `scale.z`), and the intent-layer panel root must carry
//! `NoPointerCapture` so its full-width layout anchor does not
//! block the world brush.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use top_down_2d_rts_prototype_nano_swarm::{
    background_overlay_transform,
    ui::{
        check_ui_interaction, intent_layer_panel::IntentLayerPanelRoot, NoPointerCapture,
        UiHandling,
    },
    zone_overlay_transform, MAP_HEIGHT, MAP_WIDTH, ZONE_BLOCK_SIZE,
};

/// Spawn the two overlay entities with the production component
/// bundle (`Mesh2d` + `Transform`) and the literal z values the
/// helpers must produce. The tests then assert the spawned values
/// match the helpers, so a future regression that swaps `scale.z`
/// back in for `translation.z` is caught here.
fn spawn_overlays(app: &mut App) -> (Entity, Entity) {
    app.init_resource::<Assets<Mesh>>();
    let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
    let mesh = meshes.add(Mesh::from(Rectangle::default()));
    let mesh_handle = mesh.clone();

    // Background
    let bg = app
        .world_mut()
        .spawn((
            Mesh2d(mesh),
            Transform::from_translation(Vec3::new(0.0, 0.0, -100.0)).with_scale(Vec3::new(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
                1.0,
            )),
        ))
        .id();

    // Zone overlay
    let zone = app
        .world_mut()
        .spawn((
            Mesh2d(mesh_handle),
            Transform::from_translation(Vec3::new(0.0, 0.0, -99.0)).with_scale(Vec3::new(
                MAP_WIDTH as f32 * ZONE_BLOCK_SIZE,
                MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE,
                1.0,
            )),
        ))
        .id();

    (bg, zone)
}

#[test]
fn background_overlay_translation_z_is_below_zone_overlay() {
    let mut app = App::new();
    let (bg, zone) = spawn_overlays(&mut app);

    let bg_z = app
        .world()
        .entity(bg)
        .get::<Transform>()
        .expect("background must have a Transform")
        .translation
        .z;
    let zone_z = app
        .world()
        .entity(zone)
        .get::<Transform>()
        .expect("zone must have a Transform")
        .translation
        .z;

    assert_eq!(bg_z, background_overlay_transform(1.0, 1.0).translation.z);
    assert_eq!(zone_z, zone_overlay_transform(1.0, 1.0).translation.z);
    assert!(
        zone_z > bg_z,
        "zone overlay (z={zone_z}) must draw in front of the background (z={bg_z})"
    );
    assert!(
        zone_z < 1.0,
        "zone overlay must sit below gameplay sprites at z=1"
    );
}

#[test]
fn background_and_zone_meshes_keep_scale_z_at_one() {
    let mut app = App::new();
    let (bg, zone) = spawn_overlays(&mut app);

    let bg_scale = app
        .world()
        .entity(bg)
        .get::<Transform>()
        .expect("background must have a Transform")
        .scale;
    let zone_scale = app
        .world()
        .entity(zone)
        .get::<Transform>()
        .expect("zone must have a Transform")
        .scale;

    // scale.z must stay 1.0; the old bug put a negative z on the
    // scale (mistaking it for draw order), which mirrored the
    // unit rectangle on the z axis.
    assert_eq!(bg_scale.z, 1.0);
    assert_eq!(zone_scale.z, 1.0);
    assert_eq!(bg_scale.x, MAP_WIDTH as f32 * ZONE_BLOCK_SIZE);
    assert_eq!(bg_scale.y, MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE);
    assert_eq!(zone_scale.x, MAP_WIDTH as f32 * ZONE_BLOCK_SIZE);
    assert_eq!(zone_scale.y, MAP_HEIGHT as f32 * ZONE_BLOCK_SIZE);
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
