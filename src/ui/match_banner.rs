use bevy::prelude::*;
use bevy::ui::{AlignItems, BorderRadius, JustifyContent, UiRect};

use crate::nanobot::MatchOutcome;
use crate::session::SessionRules;

use super::{ui_interaction_system::NoPointerCapture, ui_setup::FontsResource};

#[derive(Debug, Component)]
pub struct MatchBannerRoot;

#[derive(Debug, Component)]
pub struct MatchBannerText;

pub fn setup_match_banner(mut commands: Commands, fonts: Res<FontsResource>) {
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
            MatchBannerRoot,
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
                    MatchBannerText,
                ));
            });
        });
}

pub fn update_match_banner_system(
    outcome: Option<Res<MatchOutcome>>,
    rules: Res<SessionRules>,
    mut previous: Local<Option<MatchOutcome>>,
    mut roots: Query<&mut Visibility, With<MatchBannerRoot>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<MatchBannerText>>,
) {
    let presentation = outcome.as_deref().copied().unwrap_or_default();
    if *previous == Some(presentation) {
        return;
    }
    *previous = Some(presentation);
    let label = rules.outcome_label(presentation);
    for mut visibility in &mut roots {
        *visibility = if label.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for (mut text, mut color) in &mut texts {
        let next_color = match presentation {
            MatchOutcome::Winner(winner) if rules.player_swarm == Some(winner) => {
                Color::srgb(0.35, 0.95, 0.55)
            }
            MatchOutcome::Draw => Color::srgb(1.0, 0.85, 0.4),
            MatchOutcome::Winner(_) => Color::srgb(1.0, 0.35, 0.35),
            MatchOutcome::InProgress => Color::WHITE,
        };
        **text = label.as_deref().unwrap_or_default().to_string();
        color.0 = next_color;
    }
}
