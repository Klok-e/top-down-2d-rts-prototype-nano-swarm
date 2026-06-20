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

/// UI chrome that does not depend on the brush system: a single FPS counter
/// in the top-left corner. The group-select panel, merge/split buttons, and
/// zone-mode toggle are gone; the swarm intent model drives all player
/// feedback through the world itself. The visible intent-layer controls
/// live in [`crate::ui::intent_layer_panel`].
pub fn setup_ui_system(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load("fonts/fira_sans/FiraSans-Bold.ttf");
    commands.insert_resource(FontsResource { font: font.clone() });

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

#[derive(Component)]
pub struct FpsText;
