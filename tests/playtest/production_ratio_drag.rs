//! Scripted player flow for pressing, dragging, and releasing production-ratio handles.

use bevy::{prelude::*, ui::RelativeCursorPosition};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{NanobotType, ProductionRatio},
    ui::{
        production_ratio_panel::{
            production_ratio_drag_system, setup_production_ratio_panel,
            update_production_ratio_panel, HandleBoundary, ProductionRatioDragState,
            ProductionRatioHandle, ProductionRatioTrack,
        },
        FontsResource,
    },
};

#[path = "../common/mod.rs"]
mod common;

fn build_app() -> App {
    let mut app = common::minimal_app();
    app.insert_resource(FontsResource {
        font: Handle::default(),
    })
    .insert_resource(ProductionRatio::default())
    .init_resource::<ProductionRatioDragState>()
    .init_resource::<ButtonInput<MouseButton>>()
    .add_systems(Startup, setup_production_ratio_panel)
    .add_systems(
        Update,
        (production_ratio_drag_system, update_production_ratio_panel).chain(),
    );
    app.update();
    app
}

fn set_track_position(app: &mut App, x: f32) {
    let mut query = app
        .world_mut()
        .query_filtered::<&mut RelativeCursorPosition, With<ProductionRatioTrack>>();
    let mut cursor = query.single_mut(app.world_mut()).unwrap();
    cursor.normalized = Some(Vec2::new(x - 0.5, 0.0));
    cursor.cursor_over = true;
}

fn hover_worker_handle(app: &mut App) {
    let entities: Vec<_> = app
        .world_mut()
        .query::<(Entity, &ProductionRatioHandle)>()
        .iter(app.world())
        .map(|(entity, handle)| (entity, handle.0))
        .collect();
    for (entity, boundary) in entities {
        let mut handle_entity = app.world_mut().entity_mut(entity);
        let mut cursor = handle_entity.get_mut::<RelativeCursorPosition>().unwrap();
        cursor.cursor_over = boundary == HandleBoundary::WorkerEnd;
        cursor.normalized = cursor.cursor_over.then_some(Vec2::splat(0.5));
    }
}

#[test]
fn live_mouse_drag_updates_until_release_then_stops() {
    let mut app = build_app();
    hover_worker_handle(&mut app);
    set_track_position(&mut app, 0.4);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        40,
        "pressing visible Worker handle starts drag"
    );

    set_track_position(&mut app, 0.2);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        20,
        "held mouse follows live track cursor"
    );

    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    set_track_position(&mut app, 0.8);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ProductionRatio>()
            .weight(NanobotType::Worker),
        20,
        "released mouse ends pointer capture and later cursor movement does not edit ratio"
    );
}
