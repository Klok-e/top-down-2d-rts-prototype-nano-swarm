use bevy::{input::Input, prelude::*};
use rand::Rng;

use crate::{
    nanobot::{DirectMovementComponent, Nanobot, BOT_RADIUS},
    ui::{zone_button::MouseActionMode, SelectedGroupsChanged, UiHandling},
};

#[derive(Debug, Component)]
pub struct Selected {}

const MOVE_PERTURBATION_SIZE: f32 = 10.;
const BIAS_RATE: f32 = 0.5;

#[allow(clippy::too_many_arguments)]
pub fn unit_select_system(
    mut commands: Commands,
    windows: Query<&Window>,
    mouse_button_input: Res<Input<MouseButton>>,
    keyboard_input: Res<Input<KeyCode>>,
    mut nanobots: Query<(&Parent, &mut Transform), With<Nanobot>>,
    mut selected_groups: Query<(Entity, &Children), With<Selected>>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
    mut ev_select_changed: EventWriter<SelectedGroupsChanged>,
    ui_handling: Res<UiHandling>,
    mouse_mode: Res<MouseActionMode>,
) {
    // don't do anything if cursor over ui
    if ui_handling.is_pointer_over_ui {
        return;
    }

    // mouse mode must be appropriate for this system
    if *mouse_mode != MouseActionMode::GroupSelectMove {
        return;
    }

    // Get the cursor position in window coordinates
    let Some(cursor_pos) = windows.single().cursor_position() else {
        return;
    };

    // Convert the cursor position to world coordinates using viewport_to_world_2d
    let (camera_transform, camera) = camera_query.single();
    let cursor_pos_world =
        if let Some(pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) {
            pos.extend(0.0)
        } else {
            return;
        };

    // Handle left mouse button clicks
    if mouse_button_input.just_pressed(MouseButton::Left) {
        if !keyboard_input.pressed(KeyCode::ControlLeft)
            && !keyboard_input.pressed(KeyCode::ControlRight)
        {
            // Deselect all groups
            for (entity, _) in selected_groups.iter() {
                commands.entity(entity).remove::<Selected>();

                // notify other systems
                ev_select_changed.send(SelectedGroupsChanged::Deselected(entity))
            }
        }

        // Select the unit under the cursor
        for (parent, transform) in nanobots.iter_mut() {
            if (transform.translation - cursor_pos_world).length() < BOT_RADIUS {
                // Add selected tag to parent group of this nanobot
                commands.entity(parent.get()).insert(Selected {});

                // notify other systems
                ev_select_changed.send(SelectedGroupsChanged::Selected(parent.get()));
                break;
            }
        }
    }

    let mut rng = rand::thread_rng();
    // Handle right mouse button clicks
    if mouse_button_input.just_pressed(MouseButton::Right) {
        // Set the MoveDestination of the selected unit
        for (_, children) in selected_groups.iter_mut() {
            // Calculate center of mass
            let mut center_of_mass = Vec2::ZERO;
            let mut count = 0;
            for &nanobot in children {
                let (_, nanobot_transform) = nanobots.get(nanobot).expect("Invalid child");
                center_of_mass += nanobot_transform.translation.truncate();
                count += 1;
            }
            center_of_mass /= count as f32;

            // Assign move destinations based on the new center of mass and preserving relative positions
            for &nanobot in children {
                let (_, nanobot_transform) = nanobots.get(nanobot).expect("Invalid child");
                let relative_pos = nanobot_transform.translation.truncate() - center_of_mass;

                const EPSILON: f32 = 1e-3;
                let direction_to_center = if relative_pos.length() < EPSILON {
                    Vec2::ZERO
                } else {
                    (center_of_mass - nanobot_transform.translation.truncate()).normalize()
                };

                let angle: f32 = rng.gen_range(0.0..2.0 * std::f32::consts::PI);
                let perturbation = Vec2::new(angle.cos(), angle.sin());

                // Create a weighted sum of the random perturbation and the direction to the center
                let biased_perturbation =
                    perturbation * (1.0 - BIAS_RATE) + direction_to_center * BIAS_RATE;

                commands.entity(nanobot).insert(DirectMovementComponent {
                    xy: cursor_pos_world.truncate()
                        + relative_pos
                        + biased_perturbation * MOVE_PERTURBATION_SIZE,
                });
            }
        }
    }
}
