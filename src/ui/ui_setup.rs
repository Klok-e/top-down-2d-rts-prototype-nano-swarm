use bevy::prelude::*;

use super::{selected_groups_list::spawn_scrollable_list, NanobotGroupAction};

#[derive(Debug, Resource)]
pub struct FontsResource {
    pub general_text_style: TextStyle,
}

pub fn setup_ui_system(mut commands: Commands, asset_server: Res<AssetServer>) {
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

    commands.insert_resource(FontsResource {
        general_text_style: text_style.clone(),
    });

    commands
        .spawn((
            NodeBundle {
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
            },
            Interaction::default(),
        ))
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                "Selected nanobot groups",
                text_style.clone(),
            ));

            // List with hidden overflow
            spawn_scrollable_list(parent, &text_style);

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
pub struct MergeButton;
#[derive(Debug, Component)]
pub struct SplitButton;

type InteractionQuery<'a, 'b, 'c> =
    Query<'a, 'b, (Entity, &'c Interaction), (Changed<Interaction>, With<Button>)>;

pub fn button_system(
    interaction_query: InteractionQuery,
    merge_query: Query<&MergeButton>,
    split_query: Query<&SplitButton>,
    mut ev_nanobot_group_action: EventWriter<NanobotGroupAction>,
) {
    for (entity, interaction) in interaction_query.iter() {
        if let Interaction::Clicked = *interaction {
            // Handle button click
            if merge_query.get_component::<MergeButton>(entity).is_ok() {
                ev_nanobot_group_action.send(NanobotGroupAction::Merge)
            }
            if split_query.get_component::<SplitButton>(entity).is_ok() {
                ev_nanobot_group_action.send(NanobotGroupAction::Split)
            }
        }
    }
}
