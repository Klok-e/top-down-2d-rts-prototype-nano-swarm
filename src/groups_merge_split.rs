use std::collections::HashSet;

use bevy::prelude::*;

use crate::{
    nanobot::{Nanobot, NanobotGroup},
    unit_select::{NanobotGroupAction, Selected, SelectedGroupsChanged},
    GroupIdCounterResource,
};

// System that handles split and merge actions
pub fn group_action_system(
    mut commands: Commands,
    mut ev_nanobot_group_action: EventReader<NanobotGroupAction>,
    mut ev_selected_groups_changed: EventWriter<SelectedGroupsChanged>,
    _nanobots: Query<(&Parent, &mut Transform), With<Nanobot>>,
    selected_groups: Query<(Entity, &NanobotGroup, &Children), With<Selected>>,
    mut group_id_count: ResMut<GroupIdCounterResource>,
) {
    let selected_groups: Vec<_> = selected_groups.iter().collect();
    if selected_groups.is_empty() {
        return;
    }

    for action in ev_nanobot_group_action.iter() {
        match action {
            NanobotGroupAction::Merge => {
                merge(
                    &selected_groups,
                    &mut commands,
                    &mut ev_selected_groups_changed,
                    &mut group_id_count,
                );
            }
            NanobotGroupAction::Split => {
                split(
                    &selected_groups,
                    &mut commands,
                    &mut ev_selected_groups_changed,
                    &mut group_id_count,
                );
            }
        }
    }
}

fn split(
    selected_groups: &[(Entity, &NanobotGroup, &Children)],
    commands: &mut Commands<'_, '_>,
    ev_selected_groups_changed: &mut EventWriter<'_, SelectedGroupsChanged>,
    group_id_count: &mut ResMut<GroupIdCounterResource>,
) {
    for (group_entity, _, children) in selected_groups.iter() {
        // Convert children to Vec for indexed access
        let children_vec: Vec<Entity> = children.iter().cloned().collect();

        // If the group has only one nanobot, no need to split
        if children_vec.len() < 2 {
            continue;
        }

        let mid_index = children_vec.len() / 2;

        // Create two new groups for each half of the nanobots
        for i in 0..2 {
            let start_index = i * mid_index;
            let end_index = if i == 0 {
                mid_index
            } else {
                children_vec.len()
            };

            let mut new_ent = commands.spawn((
                NanobotGroup {
                    display_identifier: group_id_count.next_id(),
                },
                Selected {},
                SpatialBundle::default(),
            ));
            new_ent.push_children(&children_vec[start_index..end_index]);

            // notify other systems
            ev_selected_groups_changed.send(SelectedGroupsChanged::Selected(new_ent.id()))
        }

        // Remove the old group
        commands.entity(*group_entity).despawn();

        // notify other systems
        ev_selected_groups_changed.send(SelectedGroupsChanged::Deselected(*group_entity))
    }
}

fn merge(
    selected_groups: &Vec<(Entity, &NanobotGroup, &Children)>,
    commands: &mut Commands<'_, '_>,
    ev_selected_groups_changed: &mut EventWriter<'_, SelectedGroupsChanged>,
    group_id_count: &mut ResMut<GroupIdCounterResource>,
) {
    if selected_groups.len() < 2 {
        return;
    }

    let mut groups_to_merge = HashSet::new();
    let mut new_group_children = Vec::new();

    for (entity, _, children) in selected_groups.iter() {
        groups_to_merge.insert(entity);
        for &nanobot in *children {
            new_group_children.push(nanobot);
        }

        // delete group
        commands.entity(*entity).despawn();

        // notify other systems
        ev_selected_groups_changed.send(SelectedGroupsChanged::Deselected(*entity))
    }

    // Create new merged group
    let mut new_ent = commands.spawn((
        NanobotGroup {
            display_identifier: group_id_count.next_id(),
        },
        Selected {},
        SpatialBundle::default(),
    ));
    new_ent.push_children(&new_group_children);

    // notify other systems
    ev_selected_groups_changed.send(SelectedGroupsChanged::Selected(new_ent.id()))
}
