use std::time::Duration;

use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    time::{TimePlugin, TimeUpdateStrategy},
    window::PrimaryWindow,
};
use top_down_2d_rts_prototype_nano_swarm::fly_camera::{
    Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d,
};

fn camera_app() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((TimePlugin, Camera2dFlyPlugin))
        .add_message::<MouseWheel>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Projection::Orthographic(OrthographicProjection {
                scale: 2.0,
                ..OrthographicProjection::default_2d()
            }),
            CameraZoom2d {
                zoom: 2.0,
                zoom_speed: 0.15,
                zoom_min_max: (1.0, 30.0),
            },
            FlyCamera2d::default(),
        ))
        .id();
    app.update();
    (app, camera, window)
}

fn position(app: &App, camera: Entity) -> Vec2 {
    app.world()
        .get::<Transform>(camera)
        .unwrap()
        .translation
        .truncate()
}

fn move_cursor(app: &mut App, window: Entity, offset: Vec2) {
    let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
    let cursor = window.size() * 0.5 + offset;
    window.set_cursor_position(Some(cursor));
}

#[test]
fn wheel_preserves_the_world_point_under_the_cursor() {
    let (mut app, camera, window) = camera_app();
    move_cursor(&mut app, window, Vec2::new(220.0, -130.0));
    app.world_mut().write_message(MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: 1.0,
        window,
    });
    app.update();

    let Projection::Orthographic(projection) = app.world().get::<Projection>(camera).unwrap()
    else {
        panic!("2D camera must use an orthographic projection");
    };
    assert!(projection.scale < 2.0);
    let world_under_cursor = position(&app, camera) + Vec2::new(220.0, 130.0) * projection.scale;
    assert!(world_under_cursor.abs_diff_eq(Vec2::new(440.0, 260.0), 0.01));
}

#[test]
fn arrows_pan_and_home_recovers_the_opening_view() {
    let (mut app, camera, _) = camera_app();
    let start = position(&app, camera);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ArrowRight);
    app.update();
    assert!(position(&app, camera).x > start.x);

    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.release(KeyCode::ArrowRight);
    keys.press(KeyCode::Home);
    app.update();
    assert_eq!(position(&app, camera), start);
}

#[test]
fn middle_drag_tracks_the_grabbed_world_point() {
    let (mut app, camera, window) = camera_app();
    move_cursor(&mut app, window, Vec2::ZERO);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Middle);
    app.update();
    move_cursor(&mut app, window, Vec2::new(72.0, 33.0));
    app.update();
    assert!(position(&app, camera).abs_diff_eq(Vec2::new(-144.0, 66.0), 0.01));
}

#[test]
fn screen_edge_pans_only_while_the_pointer_is_inside_a_focused_window() {
    let (mut app, camera, window) = camera_app();
    {
        let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
        let cursor = Vec2::new(window.width() - 3.0, window.height() / 2.0);
        window.set_cursor_position(Some(cursor));
        window.focused = true;
    }
    app.update();
    assert!(position(&app, camera).x > 0.0);

    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    let before = position(&app, camera);
    app.update();
    assert_eq!(position(&app, camera), before);
}
