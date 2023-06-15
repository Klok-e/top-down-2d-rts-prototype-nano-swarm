use bevy::{
    a11y::{
        accesskit::{NodeBuilder, Role},
        AccessibilityNode,
    },
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
};

fn setup_ui_system(mut commands: Commands, asset_server: Res<AssetServer>) {
    let button_style = Style {
        margin: UiRect::all(Val::Px(5.0)),
        size: Size::new(Val::Px(100.0), Val::Px(30.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..Default::default()
    };

    let text_style = TextStyle {
        font: asset_server.load("fonts/fira_sans/FiraSans-Bold.ttf"),
        font_size: 18.0,
        color: Color::WHITE,
    };

    commands
        .spawn(NodeBundle {
            style: Style {
                size: Size::new(Val::Auto, Val::Px(300.0)),
                padding: UiRect {
                    left: Val::Px(10.),
                    right: Val::Px(10.),
                    top: Val::Px(10.),
                    bottom: Val::Px(10.),
                },
                position_type: PositionType::Absolute,
                position: UiRect {
                    left: Val::Px(10.0),
                    bottom: Val::Px(10.0),
                    ..Default::default()
                },
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
            background_color: Color::rgb(0.65, 0.65, 0.65).into(),
            ..Default::default()
        })
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                "Selected nanobot groups",
                text_style.clone(),
            ));

            // List with hidden overflow
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
                    parent
                        .spawn((
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
                        ))
                        .with_children(|parent| {
                            // List items
                            for i in 0..30 {
                                parent.spawn((
                                    TextBundle::from_section(
                                        format!("Item {i}"),
                                        TextStyle {
                                            font_size: 20.,
                                            ..text_style.clone()
                                        },
                                    ),
                                    Label,
                                    AccessibilityNode(NodeBuilder::new(Role::ListItem)),
                                ));
                            }
                        });
                });

            parent
                .spawn(ButtonBundle {
                    style: button_style.clone(),
                    background_color: BackgroundColor::from(Color::Rgba {
                        red: 0.,
                        green: 0.,
                        blue: 0.,
                        alpha: 1.,
                    }),
                    ..Default::default()
                })
                .with_children(|parent| {
                    parent.spawn(TextBundle {
                        text: Text::from_section("Merge", text_style.clone()),
                        ..Default::default()
                    });
                })
                .insert(MergeButton);

            parent
                .spawn(ButtonBundle {
                    style: button_style.clone(),
                    background_color: BackgroundColor::from(Color::Rgba {
                        red: 0.,
                        green: 0.,
                        blue: 0.,
                        alpha: 1.,
                    }),
                    ..Default::default()
                })
                .with_children(|parent| {
                    parent.spawn(TextBundle {
                        text: Text::from_section("Split", text_style.clone()),
                        ..Default::default()
                    });
                })
                .insert(SplitButton);
        });
}

#[derive(Debug, Component)]
struct MergeButton;
#[derive(Debug, Component)]
struct SplitButton;

type InteractionQuery<'a, 'b, 'c> =
    Query<'a, 'b, (Entity, &'c Interaction), (Changed<Interaction>, With<Button>)>;

fn button_system(
    interaction_query: InteractionQuery,
    merge_query: Query<&MergeButton>,
    split_query: Query<&SplitButton>,
) {
    for (entity, interaction) in interaction_query.iter() {
        if let Interaction::Clicked = *interaction {
            // Handle button click
            if merge_query.get_component::<MergeButton>(entity).is_ok() {
                println!("Merge button clicked");
                // Your merge logic here...
            }
            if split_query.get_component::<SplitButton>(entity).is_ok() {
                println!("Split button clicked");
                // Your split logic here...
            }
        }
    }
}

#[derive(Component, Default)]
struct ScrollingList {
    position: f32,
}

fn mouse_scroll(
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

#[derive(Debug, Default)]
pub struct NanoswarmUiSetupPlugin;

impl Plugin for NanoswarmUiSetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_startup_system(setup_ui_system)
            .add_system(mouse_scroll)
            .add_system(button_system);
    }
}
