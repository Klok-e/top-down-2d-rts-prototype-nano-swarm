use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::{CameraZoom2d, FlyCamera2d, reconcile_camera_tuning},
    structure_overlay::{StructureOverlay, StructureOverlayPlugin},
    tactical_overlay::{TacticalMarker, TacticalOverlayPlugin},
    tuning::Tuning,
};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn retuning_resizes_existing_bars_and_switches_overlay_layers_at_one_boundary() {
    let mut app = common::minimal_app();
    app.add_plugins((StructureOverlayPlugin, TacticalOverlayPlugin));
    let target = common::spawn_stockpile(&mut app, Vec2::ZERO, 25, 100);
    app.world_mut().spawn(CameraZoom2d {
        zoom: 3.0,
        ..default()
    });
    app.update();
    let (bar, fill) = app
        .world_mut()
        .query::<(Entity, &StructureOverlay)>()
        .iter(app.world())
        .find_map(|(entity, overlay)| (overlay.target == target).then_some((entity, overlay.fill)))
        .unwrap();
    assert_eq!(
        app.world().get::<Visibility>(bar),
        Some(&Visibility::Inherited)
    );

    let mut tuning = app.world_mut().resource_mut::<Tuning>();
    tuning.presentation.structure_bar_width = 120.0;
    tuning.presentation.structure_bar_height = 10.0;
    tuning.presentation.structure_label_gap = 30.0;
    tuning.presentation.overlay_zoom_threshold = 3.0;
    tuning.presentation.tactical_marker_screen_size = 20.0;
    app.update();

    assert_eq!(
        app.world().get::<Sprite>(bar).unwrap().custom_size,
        Some(Vec2::new(120.0, 10.0))
    );
    assert_eq!(
        app.world().get::<Sprite>(fill).unwrap().custom_size,
        Some(Vec2::new(30.0, 10.0))
    );
    assert_eq!(
        app.world().get::<Transform>(bar).unwrap().translation.y,
        62.0
    );
    assert_eq!(
        app.world().get::<Visibility>(bar),
        Some(&Visibility::Hidden)
    );
    let mut markers = app
        .world_mut()
        .query_filtered::<(&Transform, &Visibility), With<TacticalMarker>>();
    let markers = markers.iter(app.world()).collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    assert_eq!(*markers[0].1, Visibility::Inherited);
    assert_eq!(markers[0].0.scale, Vec3::splat(60.0));
}

#[test]
fn retuning_camera_sensitivity_preserves_the_current_view() {
    let mut world = World::new();
    let camera = world
        .spawn((
            FlyCamera2d {
                velocity: Vec2::new(300.0, 0.0),
                ..default()
            },
            CameraZoom2d {
                zoom: 5.0,
                zoom_speed: 0.15,
                zoom_min_max: (0.1, 20.0),
            },
            Transform::from_xyz(123.0, -456.0, 99.0),
            Projection::Orthographic(OrthographicProjection {
                scale: 5.0,
                ..OrthographicProjection::default_2d()
            }),
        ))
        .id();
    let old = Tuning::default();
    let mut next = old.clone();
    next.presentation.camera_accel = 750.0;
    next.presentation.camera_max_speed = 100.0;
    next.presentation.camera_friction = 350.0;
    next.presentation.camera_zoom_speed = 0.3;
    reconcile_camera_tuning(&mut world, &old, &next);

    let movement = world.get::<FlyCamera2d>(camera).unwrap();
    assert_eq!(
        (movement.accel, movement.max_speed, movement.friction),
        (750.0, 100.0, 350.0)
    );
    assert_eq!(movement.velocity, Vec2::new(100.0, 0.0));
    let zoom = world.get::<CameraZoom2d>(camera).unwrap();
    assert_eq!(zoom.zoom_speed, 0.3);
    assert_eq!(zoom.zoom, 5.0);
    assert_eq!(
        world.get::<Transform>(camera).unwrap().translation,
        Vec3::new(123.0, -456.0, 99.0)
    );
    let Projection::Orthographic(projection) = world.get::<Projection>(camera).unwrap() else {
        panic!("orthographic camera expected")
    };
    assert_eq!(projection.scale, 5.0);
}
