use bevy::{input::ButtonInput, prelude::*, window::PrimaryWindow};

use crate::{DEFAULT_CAMERA_ZOOM, scenario::clamp_to_map, ui::UiHandling};

use super::CameraZoom2d;

/// Pan settings and opening position for a 2D strategy camera.
#[derive(Component)]
pub struct FlyCamera2d {
    /// The speed the FlyCamera2d accelerates at.
    pub accel: f32,
    /// The maximum speed the FlyCamera can move at.
    pub max_speed: f32,
    /// The amount of deceleration to apply to the camera's motion.
    pub friction: f32,
    /// Current pan velocity, in screen pixels per second.
    pub velocity: Vec2,
    pub home: Vec2,
    /// Changes when the view jumps, so the brush can start a fresh stroke segment.
    pub view_revision: u64,
    /// Key used to move left. Defaults to <kbd>A</kbd>
    pub key_left: KeyCode,
    /// Key used to move right. Defaults to <kbd>D</kbd>
    pub key_right: KeyCode,
    /// Key used to move up. Defaults to <kbd>W</kbd>
    pub key_up: KeyCode,
    /// Key used to move down. Defaults to <kbd>S</kbd>
    pub key_down: KeyCode,
    pub enabled: bool,
}

impl Default for FlyCamera2d {
    fn default() -> Self {
        let tuning = crate::tuning::PresentationTuning::default();
        Self {
            accel: tuning.camera_accel,
            max_speed: tuning.camera_max_speed,
            friction: tuning.camera_friction,
            velocity: Vec2::ZERO,
            home: Vec2::ZERO,
            view_revision: 0,
            key_left: KeyCode::KeyA,
            key_right: KeyCode::KeyD,
            key_up: KeyCode::KeyW,
            key_down: KeyCode::KeyS,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CameraControlError {
    #[error("camera coordinates and zoom must be finite")]
    NonFinite,
    #[error("main camera must use an orthographic projection")]
    UnsupportedProjection,
}

pub fn set_camera_view(
    transform: &mut Transform,
    projection: &mut Projection,
    zoom: &mut CameraZoom2d,
    movement: &mut FlyCamera2d,
    position: Vec2,
    requested_zoom: Option<f32>,
) -> Result<(Vec2, f32), CameraControlError> {
    let applied_zoom = requested_zoom.unwrap_or(zoom.zoom);
    if !position.is_finite() || !applied_zoom.is_finite() {
        return Err(CameraControlError::NonFinite);
    }
    let Projection::Orthographic(orthographic) = projection else {
        return Err(CameraControlError::UnsupportedProjection);
    };
    let applied_zoom = applied_zoom.clamp(zoom.zoom_min_max.0, zoom.zoom_min_max.1);

    let previous_position = transform.translation.truncate();
    let applied_position = move_camera_to(transform, position);
    if applied_position != previous_position || applied_zoom != zoom.zoom {
        movement.view_revision = movement.view_revision.wrapping_add(1);
    }
    zoom.zoom = applied_zoom;
    orthographic.scale = applied_zoom;
    movement.velocity = Vec2::ZERO;
    Ok((applied_position, applied_zoom))
}

pub(super) fn move_camera_to(transform: &mut Transform, position: Vec2) -> Vec2 {
    let position = clamp_to_map(position, 0.0);
    transform.translation.x = position.x;
    transform.translation.y = position.y;
    position
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn camera_2d_movement_system(
    time: Res<Time>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Option<Res<ButtonInput<MouseButton>>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    ui: Option<Res<UiHandling>>,
    mut drag_cursor: Local<Option<Vec2>>,
    mut query: Query<(
        &mut FlyCamera2d,
        &mut Transform,
        Option<&mut Projection>,
        Option<&mut CameraZoom2d>,
    )>,
    menu: Option<Res<crate::ui::scenario_menu::ScenarioMenu>>,
    editor: Option<Res<crate::ui::tuning_panel::TuningEditor>>,
) {
    let window = primary_window.single().ok();
    let keyboard_captured = editor
        .as_ref()
        .is_some_and(|editor| editor.keyboard_captured());
    let pointer_captured = ui.is_some_and(|ui| ui.is_pointer_over_ui)
        || editor.is_some_and(|editor| editor.pointer_captured());
    let blocked = menu.is_some_and(|menu| menu.blocks_world_input)
        || window.is_some_and(|window| !window.focused);
    let cursor = window.and_then(Window::cursor_position);
    let mut drag_delta = None;
    if let Some((cursor, mouse)) = cursor
        .zip(mouse_input.as_deref())
        .filter(|(_, mouse)| !blocked && mouse.pressed(MouseButton::Middle))
    {
        if drag_cursor.is_some() || (!pointer_captured && mouse.just_pressed(MouseButton::Middle)) {
            drag_delta = Some(cursor - drag_cursor.replace(cursor).unwrap_or(cursor));
        }
    } else {
        *drag_cursor = None;
    }
    let edge_pan = if pointer_captured {
        Vec2::ZERO
    } else {
        window.zip(cursor).map_or(Vec2::ZERO, |(window, cursor)| {
            let margin = Vec2::splat(14.0).min(window.size() / 4.0);
            let low = cursor.cmplt(margin);
            let high = cursor.cmpgt(window.size() - margin);
            Vec2::new(axis(high.x, low.x), axis(low.y, high.y))
        })
    };

    for (mut options, mut transform, projection, zoom) in &mut query {
        if blocked || !options.enabled {
            options.velocity = Vec2::ZERO;
            continue;
        }
        if !keyboard_captured && keyboard_input.just_pressed(KeyCode::Home) {
            let home = options.home;
            if let Some((mut projection, mut zoom)) = projection.zip(zoom) {
                let _ = set_camera_view(
                    &mut transform,
                    &mut projection,
                    &mut zoom,
                    &mut options,
                    home,
                    Some(DEFAULT_CAMERA_ZOOM),
                );
            }
            *drag_cursor = None;
            continue;
        }

        let scale = match projection.as_deref() {
            Some(Projection::Orthographic(ortho)) => ortho.scale,
            _ => 1.0,
        };
        let position = transform.translation.truncate();
        if let Some(delta) = drag_delta {
            options.velocity = Vec2::ZERO;
            move_camera_to(
                &mut transform,
                position + Vec2::new(-delta.x, delta.y) * scale,
            );
            continue;
        }

        let keyboard_pan = if keyboard_captured {
            Vec2::ZERO
        } else {
            let pressed = |key, arrow| keyboard_input.pressed(key) || keyboard_input.pressed(arrow);
            Vec2::new(
                axis(
                    pressed(options.key_right, KeyCode::ArrowRight),
                    pressed(options.key_left, KeyCode::ArrowLeft),
                ),
                axis(
                    pressed(options.key_up, KeyCode::ArrowUp),
                    pressed(options.key_down, KeyCode::ArrowDown),
                ),
            )
        };
        let direction = (keyboard_pan + edge_pan).normalize_or_zero();
        let rate = if direction == Vec2::ZERO {
            options.friction
        } else {
            options.accel
        };
        let dt = time.delta_secs();
        options.velocity = options
            .velocity
            .move_towards(direction * options.max_speed, rate * dt);
        let displacement = options.velocity * scale * dt;
        if displacement != Vec2::ZERO {
            move_camera_to(&mut transform, position + displacement);
        }
    }
}

fn axis(positive: bool, negative: bool) -> f32 {
    (i8::from(positive) - i8::from(negative)) as f32
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::{
        app::{App, Update},
        time::{Real, TimePlugin, TimeUpdateStrategy},
    };

    use super::*;

    fn distance_after_one_second(
        frame_time: Duration,
        frames: usize,
        movement: FlyCamera2d,
    ) -> f32 {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(frame_time))
            .add_systems(Update, camera_2d_movement_system);
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .update_with_duration(Duration::ZERO);
        let camera = app.world_mut().spawn((movement, Transform::default())).id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyD);

        for _ in 0..frames {
            app.update();
        }

        app.world()
            .entity(camera)
            .get::<Transform>()
            .unwrap()
            .translation
            .x
    }

    #[test]
    fn keyboard_pan_distance_is_stable_across_frame_partitions() {
        let movement = || FlyCamera2d {
            accel: 10.0,
            max_speed: 100.0,
            friction: 0.0,
            ..Default::default()
        };
        let at_60_hz =
            distance_after_one_second(Duration::from_secs_f64(1.0 / 60.0), 60, movement());
        let at_30_hz =
            distance_after_one_second(Duration::from_secs_f64(1.0 / 30.0), 30, movement());

        assert!(
            (at_60_hz - at_30_hz).abs() < 0.2,
            "equal input duration must travel equally; 60 Hz={at_60_hz}, 30 Hz={at_30_hz}",
        );
    }

    #[test]
    fn default_keyboard_pan_covers_over_a_thousand_pixels_in_one_second() {
        let distance = distance_after_one_second(
            Duration::from_secs_f64(1.0 / 60.0),
            60,
            FlyCamera2d::default(),
        );

        assert!(
            distance > 1000.0,
            "default held input traveled {distance} screen pixels in one second",
        );
    }
}
