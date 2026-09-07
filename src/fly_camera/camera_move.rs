use bevy::{
    input::ButtonInput,
    math::{Vec2, Vec3},
    prelude::{Component, KeyCode, Projection, Query, Res, Time, Transform},
};

use super::CameraZoom2d;

/// A set of options for initializing a FlyCamera.
/// Attach this component to a [`Camera2dBundle`](https://docs.rs/bevy/0.4.0/bevy/prelude/struct.Camera2dBundle.html) bundle to control it with your keyboard.
/// # Example
/// ```no_compile
/// fn setup(mut commands: Commands) {
///   commands
///     .spawn(Camera2dBundle::default())
///     .with(FlyCamera2d::default());
/// }
#[derive(Component)]
pub struct FlyCamera2d {
    /// The speed the FlyCamera2d accelerates at.
    pub accel: f32,
    /// The maximum speed the FlyCamera can move at.
    pub max_speed: f32,
    /// The amount of deceleration to apply to the camera's motion.
    pub friction: f32,
    /// The current velocity of the FlyCamera2d. This value is always up-to-date, enforced by [FlyCameraPlugin](struct.FlyCameraPlugin.html)
    pub velocity: Vec2,
    /// Key used to move left. Defaults to <kbd>A</kbd>
    pub key_left: KeyCode,
    /// Key used to move right. Defaults to <kbd>D</kbd>
    pub key_right: KeyCode,
    /// Key used to move up. Defaults to <kbd>W</kbd>
    pub key_up: KeyCode,
    /// Key used to move forward. Defaults to <kbd>S</kbd>
    pub key_down: KeyCode,
    /// If `false`, disable keyboard control of the camera. Defaults to `true`
    pub enabled: bool,
}

impl Default for FlyCamera2d {
    fn default() -> Self {
        const MUL_2D: f32 = 10.0;
        const REFERENCE_UPDATES_PER_SECOND: f32 = 60.0;
        Self {
            accel: 3.0 * MUL_2D * REFERENCE_UPDATES_PER_SECOND,
            max_speed: 1.0 * MUL_2D * REFERENCE_UPDATES_PER_SECOND,
            friction: 1.75 * MUL_2D * REFERENCE_UPDATES_PER_SECOND,
            velocity: Vec2::ZERO,
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
) -> Result<f32, CameraControlError> {
    let applied_zoom = requested_zoom.unwrap_or(zoom.zoom);
    if !position.is_finite() || !applied_zoom.is_finite() {
        return Err(CameraControlError::NonFinite);
    }
    let Projection::Orthographic(orthographic) = projection else {
        return Err(CameraControlError::UnsupportedProjection);
    };
    let applied_zoom = applied_zoom.clamp(zoom.zoom_min_max.0, zoom.zoom_min_max.1);

    transform.translation.x = position.x;
    transform.translation.y = position.y;
    zoom.zoom = applied_zoom;
    orthographic.scale = applied_zoom;
    movement.velocity = Vec2::ZERO;
    Ok(applied_zoom)
}

pub fn camera_2d_movement_system(
    time: Res<Time>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut query: Query<(&mut FlyCamera2d, &mut Transform, Option<&Projection>)>,
    menu: Option<Res<crate::ui::scenario_menu::ScenarioMenu>>,
) {
    if menu.is_some_and(|menu| menu.blocks_world_input) {
        for (mut options, _, _) in &mut query {
            options.velocity = Vec2::ZERO;
        }
        return;
    }
    for (mut options, mut transform, ortho) in query.iter_mut() {
        let (axis_h, axis_v) = if options.enabled {
            (
                movement_axis(&keyboard_input, options.key_right, options.key_left),
                movement_axis(&keyboard_input, options.key_up, options.key_down),
            )
        } else {
            (0.0, 0.0)
        };

        let accel: Vec2 = (Vec2::X * axis_h) + (Vec2::Y * axis_v);
        let accel: Vec2 = if accel.length() != 0.0 {
            accel.normalize() * options.accel
        } else {
            Vec2::ZERO
        };

        let friction: Vec2 = if options.velocity.length() != 0.0 {
            options.velocity.normalize() * -1.0 * options.friction
        } else {
            Vec2::ZERO
        };

        options.velocity += accel * time.delta_secs();

        // clamp within max speed
        if options.velocity.length() > options.max_speed {
            options.velocity = options.velocity.normalize() * options.max_speed;
        }

        let delta_friction = friction * time.delta_secs();

        options.velocity =
            if (options.velocity + delta_friction).signum() != options.velocity.signum() {
                Vec2::ZERO
            } else {
                options.velocity + delta_friction
            };

        let scale = match ortho {
            Some(Projection::Orthographic(ortho)) => ortho.scale,
            _ => 1.,
        };
        transform.translation +=
            Vec3::new(options.velocity.x, options.velocity.y, 0.0) * scale * time.delta_secs();
    }
}

fn movement_axis(input: &Res<ButtonInput<KeyCode>>, plus: KeyCode, minus: KeyCode) -> f32 {
    let mut axis = 0.0;
    if input.pressed(plus) {
        axis += 1.0;
    }
    if input.pressed(minus) {
        axis -= 1.0;
    }
    axis
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
    fn default_keyboard_pan_keeps_a_usable_one_second_distance() {
        let distance = distance_after_one_second(
            Duration::from_secs_f64(1.0 / 60.0),
            60,
            FlyCamera2d::default(),
        );

        assert!(
            (distance - 374.375).abs() < 1.0,
            "default held input should travel about 374 screen pixels in one second, got {distance}",
        );
    }
}
