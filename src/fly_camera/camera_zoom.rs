use bevy::{
    input::mouse::MouseWheel,
    prelude::{Component, EventReader, OrthographicProjection, Query, Res},
    time::Time,
};

#[derive(Default, Component)]
pub struct CameraZoom2d {
    /// The speed at which the camera zooms in and out.
    pub zoom_speed: f32,
    /// The maximum and minimum zoom levels allowed.
    pub zoom_min_max: (f32, f32),
    /// The current zoom level.
    pub zoom: f32,
}

pub fn camera_2d_zoom_system(
    time: Res<Time>,
    mut mouse_wheel_event_reader: EventReader<MouseWheel>,
    mut query: Query<(&mut CameraZoom2d, &mut OrthographicProjection)>,
) {
    for (mut zoom, mut ortho) in query.iter_mut() {
        for event in mouse_wheel_event_reader.read() {
            // Update the zoom speed based on the current zoom level
            let dynamic_zoom_speed = zoom.zoom_speed * zoom.zoom;

            zoom.zoom -= event.y * dynamic_zoom_speed * time.delta_seconds();
            zoom.zoom = zoom.zoom.clamp(zoom.zoom_min_max.0, zoom.zoom_min_max.1); // limit the zoom level

            // Update the camera's scale
            ortho.scale = zoom.zoom;
        }
    }
}
