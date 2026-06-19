use bevy::{prelude::*, ui::RelativeCursorPosition};

use super::{
    button_bg_interaction::ButtonBgInteractiveComponent, consts::NORMAL_BUTTON, fps_count::FpsText,
    selected_groups_list::spawn_scrollable_list, zone_button::ZoneButton, NanobotGroupAction,
};

#[derive(Debug, Resource, Clone)]
pub struct FontsResource {
    pub font: Handle<Font>,
}

fn text_components(
    text: impl Into<String>,
    font: Handle<Font>,
    font_size: f32,
) -> (Text, TextFont, TextColor) {
    (
        Text::new(text),
        TextFont {
            font,
            font_size,
            ..default()
        },
        TextColor(Color::WHITE),
    )
}

pub fn setup_ui_system(mut commands: Commands, asset_server: Res<AssetServer>) {
    let button_node = Node {
        margin: UiRect::all(Val::Px(5.0)),
        width: Val::Px(100.0),
        height: Val::Px(30.0),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };

    let font = asset_server.load("fonts/fira_sans/FiraSans-Bold.ttf");
    commands.insert_resource(FontsResource { font: font.clone() });

    commands
        .spawn((
            Node {
                width: Val::Auto,
                height: Val::Px(300.0),
                padding: UiRect {
                    left: Val::Px(10.),
                    right: Val::Px(10.),
                    top: Val::Px(10.),
                    bottom: Val::Px(10.),
                },
                position_type: PositionType::Absolute,
                left: Val::Px(10.0),
                bottom: Val::Px(10.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgb(0.65, 0.65, 0.65)),
            Interaction::default(),
            RelativeCursorPosition::default(),
        ))
        .with_children(|parent| {
            parent.spawn(text_components(
                "Selected nanobot groups",
                font.clone(),
                18.0,
            ));

            spawn_scrollable_list(parent, font.clone());

            parent
                .spawn((
                    Button,
                    button_node.clone(),
                    BackgroundColor(NORMAL_BUTTON),
                    MergeButton,
                    ButtonBgInteractiveComponent,
                ))
                .with_children(|parent| {
                    parent.spawn(text_components("Merge", font.clone(), 18.0));
                });

            parent
                .spawn((
                    Button,
                    button_node.clone(),
                    BackgroundColor(NORMAL_BUTTON),
                    SplitButton,
                    ButtonBgInteractiveComponent,
                ))
                .with_children(|parent| {
                    parent.spawn(text_components("Split", font.clone(), 18.0));
                });

            parent
                .spawn((
                    Button,
                    button_node.clone(),
                    BackgroundColor(NORMAL_BUTTON),
                    ZoneButton,
                ))
                .with_children(|parent| {
                    parent.spawn(text_components("Zone", font.clone(), 18.0));
                });
        });

    commands.spawn((
        text_components("FPS: --", font, 18.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(5.0),
            left: Val::Px(5.0),
            ..default()
        },
        FpsText,
    ));
}

#[derive(Debug, Component)]
pub struct MergeButton;
#[derive(Debug, Component)]
pub struct SplitButton;

#[allow(clippy::type_complexity)]
pub fn button_system(
    interaction_query: Query<(Entity, &Interaction), (Changed<Interaction>, With<Button>)>,
    merge_query: Query<&MergeButton>,
    split_query: Query<&SplitButton>,
    mut ev_nanobot_group_action: MessageWriter<NanobotGroupAction>,
) {
    for (entity, interaction) in interaction_query.iter() {
        if let Interaction::Pressed = *interaction {
            if merge_query.get(entity).is_ok() {
                ev_nanobot_group_action.write(NanobotGroupAction::Merge);
            }
            if split_query.get(entity).is_ok() {
                ev_nanobot_group_action.write(NanobotGroupAction::Split);
            }
        }
    }
}
