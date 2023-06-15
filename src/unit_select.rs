use bevy::{input::Input, prelude::*};

use crate::nanobot::{MoveDestination, Nanobot, BOT_RADIUS};

#[derive(Debug, Component)]
pub struct Selected {}

pub fn unit_select_system(
    mut commands: Commands,
    windows: Query<&Window>,
    mouse_button_input: Res<Input<MouseButton>>,
    mut nanobots: Query<(&Parent, &mut Transform), With<Nanobot>>,
    mut selected_groups: Query<(Entity, &Children), With<Selected>>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
) {
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
        // Deselect all groups
        for (entity, _) in selected_groups.iter() {
            commands.entity(entity).remove::<Selected>();
        }

        // Select the unit under the cursor
        for (parent, transform) in nanobots.iter_mut() {
            if (transform.translation - cursor_pos_world).length() < BOT_RADIUS {
                // Add selected tag to parent group of this nanobot
                commands.entity(parent.get()).insert(Selected {});
                break;
            }
        }
    }

    // Handle right mouse button clicks
    if mouse_button_input.just_pressed(MouseButton::Right) {
        // Set the MoveDestination of the selected unit
        for (_, children) in selected_groups.iter_mut() {
            for &nanobot in children {
                commands.entity(nanobot).insert(MoveDestination {
                    xy: cursor_pos_world.truncate(),
                });
            }
        }
    }
}
