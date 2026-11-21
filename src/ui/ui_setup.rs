use bevy::prelude::*;

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

/// Load the shared UI font and create the optional FPS counter.
pub fn setup_ui_system(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load("fonts/fira_sans/FiraSans-Bold.ttf");
    commands.insert_resource(FontsResource { font: font.clone() });

    commands.spawn((
        text_components("FPS: --", font, 16.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(48.0),
            left: Val::Px(5.0),
            ..default()
        },
        Visibility::Hidden,
        FpsText,
    ));
}

#[derive(Component)]
pub struct FpsText;
