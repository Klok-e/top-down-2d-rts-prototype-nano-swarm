use bevy::{
    ecs::hierarchy::ChildSpawnerCommands,
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
};

#[derive(Component, Default)]
pub struct ScrollingList {
    position: f32,
}

#[derive(Debug, Component)]
pub struct SelectedGroupsList;

pub fn spawn_scrollable_list(parent: &mut ChildSpawnerCommands<'_>, _font: Handle<Font>) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_self: AlignSelf::Stretch,
                height: Val::Percent(50.0),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgb(0.10, 0.10, 0.10)),
            Interaction::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    max_width: Val::Auto,
                    max_height: Val::Auto,
                    align_items: AlignItems::Center,
                    ..default()
                },
                ScrollingList::default(),
                SelectedGroupsList,
            ));
        });
}

pub fn mouse_scroll(
    mut mouse_wheel_events: MessageReader<MouseWheel>,
    mut query_list: Query<(&mut ScrollingList, &mut Node, &ChildOf, &ComputedNode)>,
    interaction_nodes: Query<&Interaction>,
    query_node: Query<&ComputedNode>,
) {
    for mouse_wheel_event in mouse_wheel_events.read() {
        for (mut scrolling_list, mut node, parent, list_node) in &mut query_list {
            if *interaction_nodes
                .get(parent.parent())
                .expect("All scroll lists must have interactable parents")
                != Interaction::Hovered
            {
                continue;
            }

            let items_height = list_node.size().y;
            let container_height = query_node.get(parent.parent()).unwrap().size().y;
            let max_scroll = (items_height - container_height).max(0.);

            let dy = match mouse_wheel_event.unit {
                MouseScrollUnit::Line => mouse_wheel_event.y * 20.,
                MouseScrollUnit::Pixel => mouse_wheel_event.y,
            };

            scrolling_list.position += dy;
            scrolling_list.position = scrolling_list.position.clamp(-max_scroll, 0.);
            node.top = Val::Px(scrolling_list.position);
        }
    }
}
