use bevy::{input::Input, prelude::*};

use crate::nanobot::{MoveDestination, Nanobot, BOT_RADIUS};

#[derive(Debug, Component)]
pub struct Selected {}

pub fn unit_select_system(
    mut commands: Commands,
    windows: Query<&Window>,
    mouse_button_input: Res<Input<MouseButton>>,
    mut query_all: Query<(Entity, &mut Transform), With<Nanobot>>,
    mut query_selected: Query<Entity, With<Selected>>,
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
        // Deselect the currently selected unit
        for entity in query_selected.iter() {
            commands.entity(entity).remove::<Selected>();
        }

        // Select the unit under the cursor
        for (entity, transform) in query_all.iter_mut() {
            if (transform.translation - cursor_pos_world).length() < BOT_RADIUS {
                commands.entity(entity).insert(Selected {});
                break;
            }
        }
    }

    // Handle right mouse button clicks
    if mouse_button_input.just_pressed(MouseButton::Right) {
        // Set the MoveDestination of the selected unit
        for entity in query_selected.iter_mut() {
            commands.entity(entity).insert(MoveDestination {
                xy: cursor_pos_world.truncate(),
            });
        }
    }
}
