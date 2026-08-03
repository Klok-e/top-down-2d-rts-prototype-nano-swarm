use bevy::prelude::*;
use bevy::ui::{AlignItems, BorderRadius, JustifyContent, UiRect};

use crate::nanobot::MatchOutcome;

use super::{ui_interaction_system::NoPointerCapture, ui_setup::FontsResource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapsePresentation {
    Hidden,
    Victory,
    Defeat,
}

pub fn collapse_presentation(outcome: MatchOutcome) -> CollapsePresentation {
    match outcome {
        MatchOutcome::InProgress => CollapsePresentation::Hidden,
        MatchOutcome::Victory => CollapsePresentation::Victory,
        MatchOutcome::Defeat => CollapsePresentation::Defeat,
    }
}

#[derive(Debug, Component)]
pub struct CollapseBannerRoot;

#[derive(Debug, Component)]
pub struct CollapseBannerText;

pub fn setup_collapse_banner(mut commands: Commands, fonts: Res<FontsResource>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            CollapseBannerRoot,
            NoPointerCapture,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(48.0), Val::Px(28.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.025, 0.03, 0.94)),
            ))
            .with_children(|card| {
                card.spawn((
                    Text::new(""),
                    TextFont {
                        font: fonts.font.clone(),
                        font_size: 42.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TextLayout::new_with_justify(Justify::Center),
                    CollapseBannerText,
                ));
            });
        });
}

pub fn update_collapse_banner_system(
    outcome: Option<Res<MatchOutcome>>,
    mut previous: Local<Option<CollapsePresentation>>,
    mut roots: Query<&mut Visibility, With<CollapseBannerRoot>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<CollapseBannerText>>,
) {
    let presentation = collapse_presentation(outcome.as_deref().copied().unwrap_or_default());
    if *previous == Some(presentation) {
        return;
    }
    *previous = Some(presentation);
    for mut visibility in &mut roots {
        *visibility = if presentation == CollapsePresentation::Hidden {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    for (mut text, mut color) in &mut texts {
        let (value, next_color) = match presentation {
            CollapsePresentation::Hidden => ("", Color::WHITE),
            CollapsePresentation::Victory => (
                "VICTORY\nOpponent Production Collapsed",
                Color::srgb(0.35, 0.95, 0.55),
            ),
            CollapsePresentation::Defeat => {
                ("DEFEAT\nProduction Collapse", Color::srgb(1.0, 0.35, 0.35))
            }
        };
        **text = value.to_string();
        color.0 = next_color;
    }
}
