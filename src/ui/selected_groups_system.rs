use bevy::{
    a11y::{
        accesskit::{NodeBuilder, Role},
        AccessibilityNode,
    },
    prelude::{
        BuildChildren, Children, Commands, Component, Entity, EventReader, Label, Query, Res,
        TextBundle, With,
    },
    text::TextStyle,
};

use crate::nanobot::NanobotGroup;

use super::{FontsResource, SelectedGroupsChanged, SelectedGroupsList};

#[derive(Debug, Component)]
pub struct SelectedGroupReference(Entity);

pub fn update_selected_nanobot_groups_system(
    mut commands: Commands,
    fonts: Res<FontsResource>,
    mut ev_select_changed: EventReader<SelectedGroupsChanged>,
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
                            TextBundle::from_section(
                                format!("Group {}", display.id),
                                TextStyle {
                                    font_size: 20.,
                                    ..fonts.general_text_style.clone()
                                },
                            ),
                            Label,
                            AccessibilityNode(NodeBuilder::new(Role::ListItem)),
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
                                .get(**x)
                                .expect("Children must be valid")
                                .0
                                == *deselected_ent
                        })
                        .expect("Can't deselect not selected entity");
                    commands.entity(ent).remove_children(&[*child]);
                    commands.entity(*child).despawn();
                }
            }
        }
    }
}
