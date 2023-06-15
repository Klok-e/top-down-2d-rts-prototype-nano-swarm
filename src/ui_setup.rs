use bevy::prelude::*;

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
        font_size: 30.0,
        color: Color::WHITE,
    };

    commands
        .spawn(NodeBundle {
            style: Style {
                size: Size::new(Val::Px(200.0), Val::Px(400.0)),
                position_type: PositionType::Absolute,
                position: UiRect {
                    left: Val::Px(10.0),
                    bottom: Val::Px(10.0),
                    ..Default::default()
                },
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
            ..Default::default()
        })
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                "Selected nanobot groups",
                text_style.clone(),
            ));

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

pub type InteractionQuery<'a, 'b, 'c> =
    Query<'a, 'b, (Entity, &'c Interaction), (Changed<Interaction>, With<Button>)>;

pub fn button_system(
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
