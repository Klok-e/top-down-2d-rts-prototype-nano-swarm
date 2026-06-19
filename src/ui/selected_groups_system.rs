use bevy::prelude::*;

use crate::nanobot::NanobotGroup;

use super::{FontsResource, SelectedGroupsChanged, SelectedGroupsList};

#[derive(Debug, Component)]
pub struct SelectedGroupReference(Entity);

pub fn update_selected_nanobot_groups_system(
    mut commands: Commands,
    fonts: Res<FontsResource>,
    mut ev_select_changed: MessageReader<SelectedGroupsChanged>,
    selected_groups_lists: Query<(Entity, Option<&Children>), With<SelectedGroupsList>>,
    selected_groups_list_children: Query<&SelectedGroupReference>,
    groups: Query<&NanobotGroup>,
) {
    for ev in ev_select_changed.read() {
        for (ent, children) in &selected_groups_lists {
            match ev {
                SelectedGroupsChanged::Selected(selected_ent) => {
                    let display = groups
                        .get(*selected_ent)
                        .expect("Event must have a valid entity");
                    commands.entity(ent).with_children(|parent| {
                        parent.spawn((
                            Text::new(format!("Group {}", display.id)),
                            TextFont {
                                font: fonts.font.clone(),
                                font_size: 20.,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            Label,
                            SelectedGroupReference(*selected_ent),
                        ));
                    });
                }
                SelectedGroupsChanged::Deselected(deselected_ent) => {
                    let child = children
                        .expect("Can't deselect something when nothing is selected")
                        .iter()
                        .find(|x| {
                            selected_groups_list_children
                                .get(*x)
                                .expect("Children must be valid")
                                .0
                                == *deselected_ent
                        })
                        .expect("Can't deselect not selected entity");
                    commands.entity(ent).detach_children(&[child]);
                    commands.entity(child).despawn();
                }
            }
        }
    }
}
