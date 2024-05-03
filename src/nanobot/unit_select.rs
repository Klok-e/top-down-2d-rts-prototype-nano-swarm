use bevy::{
    input::ButtonInput,
    math::{vec2, vec3},
    prelude::*,
    sprite::MaterialMesh2dBundle,
};
use rand::{rngs::ThreadRng, Rng};

use crate::{
    materials::BackgroundMaterial,
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
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut nanobots: Query<(&Parent, &mut Transform), With<Nanobot>>,
    selected_groups: Query<(Entity, &Children), With<Selected>>,
    camera_query: Query<(&GlobalTransform, &Camera)>,
    mut ev_select_changed: EventWriter<SelectedGroupsChanged>,
    ui_handling: Res<UiHandling>,
    mouse_mode: Res<MouseActionMode>,
    mut res_selection_start: Local<Option<Vec3>>,
    meshes: ResMut<Assets<Mesh>>,
    bg_mats: ResMut<Assets<ColorMaterial>>,
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

    if mouse_button_input.just_pressed(MouseButton::Left) {
        handle_mouse_click(
            keyboard_input,
            &selected_groups,
            &mut commands,
            &mut ev_select_changed,
            &mut nanobots,
            cursor_pos_world,
        );

        *res_selection_start = Some(cursor_pos_world);
    }

    handle_rect_selection(
        res_selection_start,
        cursor_pos_world,
        &mut commands,
        meshes,
        bg_mats,
        &mouse_button_input,
        &mut nanobots,
        ev_select_changed,
    );

    let rng = rand::thread_rng();
    // Handle right mouse button clicks
    if mouse_button_input.just_pressed(MouseButton::Right) {
        // Set the MoveDestination of the selected unit
        add_direct_movement(selected_groups, nanobots, rng, commands, cursor_pos_world);
    }
}

fn handle_rect_selection(
    mut res_selection_start: Local<Option<Vec3>>,
    cursor_pos_world: Vec3,
    commands: &mut Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut bg_mats: ResMut<Assets<ColorMaterial>>,
    mouse_button_input: &Res<ButtonInput<MouseButton>>,
    nanobots: &mut Query<(&Parent, &mut Transform), With<Nanobot>>,
    mut ev_select_changed: EventWriter<SelectedGroupsChanged>,
) {
    if let Some(selection_start) = *res_selection_start {
        let bottom_left = vec2(
            selection_start.x.min(cursor_pos_world.x),
            selection_start.y.min(cursor_pos_world.y),
        );
        let top_right = vec2(
            selection_start.x.max(cursor_pos_world.x),
            selection_start.y.max(cursor_pos_world.y),
        );
        let rectangle = Rect::from_corners(bottom_left, top_right);

        let scale = vec3(rectangle.width(), rectangle.height(), 1.0);
        let translation = vec3(
            bottom_left.x + rectangle.width() * 0.5,
            top_right.y - rectangle.height() * 0.5,
            0.0,
        );

        commands.spawn(MaterialMesh2dBundle {
            mesh: meshes.add(Mesh::from(Rectangle::default())).into(),
            material: bg_mats.add(ColorMaterial::from(Color::LIME_GREEN)),
            transform: Transform::from_translation(translation).with_scale(scale),
            ..default()
        });

        if mouse_button_input.just_released(MouseButton::Left) {
            let mut to_add_selected = vec![];
            for (parent, transform) in nanobots.iter_mut() {
                if rectangle.contains(transform.translation.xy())
                    && !to_add_selected.contains(&parent.get())
                {
                    to_add_selected.push(parent.get());
                }
            }

            for entity in to_add_selected {
                commands.entity(entity).insert(Selected {});

                ev_select_changed.send(SelectedGroupsChanged::Selected(entity));
            }

            *res_selection_start = None;
        }
    }
}

fn handle_mouse_click(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    selected_groups: &Query<(Entity, &Children), With<Selected>>,
    commands: &mut Commands,
    ev_select_changed: &mut EventWriter<SelectedGroupsChanged>,
    nanobots: &mut Query<(&Parent, &mut Transform), With<Nanobot>>,
    cursor_pos_world: Vec3,
) {
    if !keyboard_input.pressed(KeyCode::ControlLeft)
        && !keyboard_input.pressed(KeyCode::ControlRight)
    {
        // Deselect all groups
        for (entity, _) in selected_groups.iter() {
            commands.entity(entity).remove::<Selected>();

            // notify other systems
            ev_select_changed.send(SelectedGroupsChanged::Deselected(entity));
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

fn add_direct_movement(
    mut selected_groups: Query<(Entity, &Children), With<Selected>>,
    nanobots: Query<(&Parent, &mut Transform), With<Nanobot>>,
    mut rng: ThreadRng,
    mut commands: Commands,
    cursor_pos_world: Vec3,
) {
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
