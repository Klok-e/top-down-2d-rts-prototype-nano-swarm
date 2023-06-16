use bevy::{
    a11y::{
        accesskit::{NodeBuilder, Role},
        AccessibilityNode,
    },
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::{
        default, BuildChildren, ChildBuilder, Color, Component, EventReader, NodeBundle, Parent,
        Query,
    },
    text::TextStyle,
    ui::{AlignItems, AlignSelf, FlexDirection, Interaction, Node, Overflow, Size, Style, Val},
};

#[derive(Component, Default)]
pub struct ScrollingList {
    position: f32,
}

#[derive(Debug, Component)]
pub struct SelectedGroupsList;

pub fn spawn_scrollable_list(parent: &mut ChildBuilder<'_, '_, '_>, _text_style: &TextStyle) {
    parent
        .spawn(NodeBundle {
            style: Style {
                flex_direction: FlexDirection::Column,
                align_self: AlignSelf::Stretch,
                size: Size::height(Val::Percent(50.0)),
                overflow: Overflow::Hidden,
                ..default()
            },
            background_color: Color::rgb(0.10, 0.10, 0.10).into(),
            ..default()
        })
        .insert(Interaction::default())
        .with_children(|parent| {
            // Moving panel
            parent.spawn((
                NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Column,
                        max_size: Size::UNDEFINED,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ..default()
                },
                ScrollingList::default(),
                AccessibilityNode(NodeBuilder::new(Role::List)),
                SelectedGroupsList,
            ));
        });
}

pub fn mouse_scroll(
    mut mouse_wheel_events: EventReader<MouseWheel>,
    mut query_list: Query<(&mut ScrollingList, &mut Style, &Parent, &Node)>,
    interaction_nodes: Query<&Interaction>,
    query_node: Query<&Node>,
) {
    for mouse_wheel_event in mouse_wheel_events.iter() {
        for (mut scrolling_list, mut style, parent, list_node) in &mut query_list {
            if *interaction_nodes
                .get(parent.get())
                .expect("All scroll lists must have interactable parents")
                != Interaction::Hovered
            {
                continue;
            }

            let items_height = list_node.size().y;
            let container_height = query_node.get(parent.get()).unwrap().size().y;

            let max_scroll = (items_height - container_height).max(0.);

            let dy = match mouse_wheel_event.unit {
                MouseScrollUnit::Line => mouse_wheel_event.y * 20.,
                MouseScrollUnit::Pixel => mouse_wheel_event.y,
            };

            scrolling_list.position += dy;
            scrolling_list.position = scrolling_list.position.clamp(-max_scroll, 0.);
            style.position.top = Val::Px(scrolling_list.position);
        }
    }
}
