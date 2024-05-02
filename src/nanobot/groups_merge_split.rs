use bevy::{prelude::*, utils::HashSet};

use crate::{
    nanobot::NanobotGroup,
    ui::{NanobotGroupAction, SelectedGroupsChanged},
    zones::{ZoneChangedEvent, ZoneChangedKind, ZoneComponent, ZonePointData},
};

use super::{GroupIdCounterResource, NanobotGroupBundle, Selected};

// System that handles split and merge actions
pub fn group_action_system(
    mut commands: Commands,
    mut ev_nanobot_group_action: EventReader<NanobotGroupAction>,
    mut ev_selected_groups_changed: EventWriter<SelectedGroupsChanged>,
    selected_groups: Query<(Entity, &NanobotGroup, &Children, &ZoneComponent), With<Selected>>,
    mut group_id_count: ResMut<GroupIdCounterResource>,
    mut ev_zone_changed: EventWriter<ZoneChangedEvent>,
) {
    let selected_groups: Vec<_> = selected_groups.iter().collect();
    if selected_groups.is_empty() {
        return;
    }

    for action in ev_nanobot_group_action.read() {
        match action {
            NanobotGroupAction::Merge => {
                merge(
                    &selected_groups,
                    &mut commands,
                    &mut ev_selected_groups_changed,
                    &mut group_id_count,
                    &mut ev_zone_changed,
                );
            }
            NanobotGroupAction::Split => {
                split(
                    &selected_groups,
                    &mut commands,
                    &mut ev_selected_groups_changed,
                    &mut group_id_count,
                    &mut ev_zone_changed,
                );
            }
        }
    }
}

fn split(
    selected_groups: &[(Entity, &NanobotGroup, &Children, &ZoneComponent)],
    commands: &mut Commands<'_, '_>,
    ev_selected_groups_changed: &mut EventWriter<'_, SelectedGroupsChanged>,
    group_id_count: &mut ResMut<GroupIdCounterResource>,
    ev_zone_changed: &mut EventWriter<ZoneChangedEvent>,
) {
    for (group_entity, group, children, zone) in selected_groups.iter() {
        // Convert children to Vec for indexed access
        let children_vec: Vec<Entity> = children.iter().cloned().collect();

        // If the group has only one nanobot, no need to split
        if children_vec.len() < 2 {
            continue;
        }

        // remove all points from zone
        for point in &zone.zone_points {
            ev_zone_changed.send(ZoneChangedEvent {
                point: *point,
                zone_color: zone.zone_color,
                zone_id: group.id as u32,
                kind: ZoneChangedKind::PointRemoved,
            });
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

            // prepare group for creation
            let id = group_id_count.next_id();
            let nanobot_group_bundle = NanobotGroupBundle {
                group: NanobotGroup { id },
                zone: ZoneComponent {
                    zone_color: ZonePointData::id_to_zone(id as u32),
                    zone_points: zone.zone_points.clone(),
                },
                ..default()
            };

            // add all points to new zone
            for point in &zone.zone_points {
                ev_zone_changed.send(ZoneChangedEvent {
                    point: *point,
                    zone_color: nanobot_group_bundle.zone.zone_color,
                    zone_id: nanobot_group_bundle.group.id as u32,
                    kind: ZoneChangedKind::PointAdded,
                });
            }

            // spawn entity
            let mut new_ent = commands.spawn((nanobot_group_bundle, Selected {}));
            new_ent.push_children(&children_vec[start_index..end_index]);

            // notify other systems
            ev_selected_groups_changed.send(SelectedGroupsChanged::Selected(new_ent.id()));
        }

        // Remove the old group
        commands.entity(*group_entity).despawn();

        // notify other systems
        ev_selected_groups_changed.send(SelectedGroupsChanged::Deselected(*group_entity));
    }
}

fn merge(
    selected_groups: &Vec<(Entity, &NanobotGroup, &Children, &ZoneComponent)>,
    commands: &mut Commands<'_, '_>,
    ev_selected_groups_changed: &mut EventWriter<'_, SelectedGroupsChanged>,
    group_id_count: &mut ResMut<GroupIdCounterResource>,
    ev_zone_changed: &mut EventWriter<ZoneChangedEvent>,
) {
    if selected_groups.len() < 2 {
        return;
    }

    let mut groups_to_merge = Vec::new();
    let mut new_group_children = Vec::new();
    let mut new_zone_points = HashSet::new();
    for (entity, group, children, zone) in selected_groups.iter() {
        groups_to_merge.push(entity);
        for &nanobot in *children {
            new_group_children.push(nanobot);
        }

        // remove all points from zone
        for point in &zone.zone_points {
            ev_zone_changed.send(ZoneChangedEvent {
                point: *point,
                zone_color: zone.zone_color,
                zone_id: group.id as u32,
                kind: ZoneChangedKind::PointRemoved,
            });
            new_zone_points.insert(*point);
        }
    }

    // Create new merged group
    let id = group_id_count.next_id();
    let nanobot_group_bundle = NanobotGroupBundle {
        group: NanobotGroup { id },
        zone: ZoneComponent {
            zone_color: ZonePointData::id_to_zone(id as u32),
            zone_points: new_zone_points,
        },
        ..default()
    };

    // add all points to new zone
    for point in &nanobot_group_bundle.zone.zone_points {
        ev_zone_changed.send(ZoneChangedEvent {
            point: *point,
            zone_color: nanobot_group_bundle.zone.zone_color,
            zone_id: nanobot_group_bundle.group.id as u32,
            kind: ZoneChangedKind::PointAdded,
        });
    }

    // spawn entity
    let mut new_ent = commands.spawn((nanobot_group_bundle, Selected {}));
    new_ent.push_children(&new_group_children);

    // notify other systems
    ev_selected_groups_changed.send(SelectedGroupsChanged::Selected(new_ent.id()));

    for group_merged in groups_to_merge {
        // delete group
        commands.entity(*group_merged).despawn();

        // notify other systems
        ev_selected_groups_changed.send(SelectedGroupsChanged::Deselected(*group_merged));
    }
}
