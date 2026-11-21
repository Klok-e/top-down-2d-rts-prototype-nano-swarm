use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
};

use crate::{DEFAULT_CAMERA_ZOOM, ui::UiHandling};

use super::camera_move::move_camera_to;

#[derive(Component)]
pub struct CameraZoom2d {
    /// Fraction of the current zoom applied per wheel step.
    pub zoom_speed: f32,
    /// The minimum and maximum zoom levels allowed.
    pub zoom_min_max: (f32, f32),
    /// The current zoom level.
    pub zoom: f32,
}

impl Default for CameraZoom2d {
    fn default() -> Self {
        Self {
            zoom_speed: crate::tuning::PresentationTuning::default().camera_zoom_speed,
            zoom_min_max: (1.0, 30.0),
            zoom: DEFAULT_CAMERA_ZOOM,
        }
    }
}

pub fn camera_2d_zoom_system(
    mut mouse_wheel_event_reader: MessageReader<MouseWheel>,
    mut query: Query<(&mut CameraZoom2d, &mut Projection, &mut Transform)>,
    windows: Query<&Window>,
    ui: Option<Res<UiHandling>>,
    menu: Option<Res<crate::ui::scenario_menu::ScenarioMenu>>,
    editor: Option<Res<crate::ui::tuning_panel::TuningEditor>>,
) {
    if menu.is_some_and(|menu| menu.blocks_world_input)
        || editor.is_some_and(|editor| editor.pointer_captured())
        || ui.is_some_and(|ui| ui.is_pointer_over_ui)
    {
        mouse_wheel_event_reader.clear();
        return;
    }
    for event in mouse_wheel_event_reader.read() {
        let delta = match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
        };
        if !delta.is_finite() || delta == 0.0 {
            continue;
        }
        let cursor_offset = windows
            .get(event.window)
            .ok()
            .and_then(|window| {
                window
                    .cursor_position()
                    .map(|cursor| cursor - window.size() / 2.0)
            })
            .unwrap_or(Vec2::ZERO);
        let cursor_offset = Vec2::new(cursor_offset.x, -cursor_offset.y);
        for (mut zoom, mut projection, mut transform) in &mut query {
            let Projection::Orthographic(ortho) = &mut *projection else {
                continue;
            };
            let old_scale = ortho.scale;
            let scale = (zoom.zoom - delta * zoom.zoom_speed * zoom.zoom)
                .clamp(zoom.zoom_min_max.0, zoom.zoom_min_max.1);
            let position = transform.translation.truncate() + cursor_offset * (old_scale - scale);
            move_camera_to(&mut transform, position);
            ortho.scale = scale;
            zoom.zoom = scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::{
        app::{App, Update},
        camera::OrthographicProjection,
        input::mouse::MouseScrollUnit,
        prelude::Entity,
        time::{Real, Time, TimePlugin, TimeUpdateStrategy},
    };

    use super::*;

    fn zoom_after_wheel_event(frame_time: Duration, unit: MouseScrollUnit, delta: f32) -> f32 {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .add_message::<MouseWheel>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(frame_time))
            .add_systems(Update, camera_2d_zoom_system);
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .update_with_duration(Duration::ZERO);
        let camera = app
            .world_mut()
            .spawn((
                CameraZoom2d {
                    zoom_speed: 0.1,
                    zoom_min_max: (1.0, 100.0),
                    zoom: 10.0,
                },
                Projection::Orthographic(OrthographicProjection {
                    scale: 10.0,
                    ..OrthographicProjection::default_2d()
                }),
                Transform::default(),
            ))
            .id();
        app.world_mut().write_message(MouseWheel {
            unit,
            x: 0.0,
            y: delta,
            window: Entity::PLACEHOLDER,
        });

        app.update();

        app.world()
            .entity(camera)
            .get::<CameraZoom2d>()
            .unwrap()
            .zoom
    }

    #[test]
    fn wheel_zoom_step_is_independent_of_frame_duration() {
        let at_60_hz = zoom_after_wheel_event(
            Duration::from_secs_f64(1.0 / 60.0),
            MouseScrollUnit::Line,
            1.0,
        );
        let at_30_hz = zoom_after_wheel_event(
            Duration::from_secs_f64(1.0 / 30.0),
            MouseScrollUnit::Line,
            1.0,
        );

        assert_eq!(at_60_hz, at_30_hz);
    }

    #[test]
    fn line_and_pixel_wheel_units_have_equivalent_zoom_steps() {
        let frame_time = Duration::from_secs_f64(1.0 / 60.0);
        let line = zoom_after_wheel_event(frame_time, MouseScrollUnit::Line, 1.0);
        let pixels = zoom_after_wheel_event(
            frame_time,
            MouseScrollUnit::Pixel,
            MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
        );

        assert_eq!(line, pixels);
    }
}
