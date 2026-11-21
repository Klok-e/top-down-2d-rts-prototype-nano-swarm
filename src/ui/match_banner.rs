use bevy::prelude::*;
use bevy::ui::{AlignItems, BorderRadius, FocusPolicy, JustifyContent, UiRect};

use crate::nanobot::{MatchOutcome, swarm_presentation_color};
use crate::session::SessionRules;
use crate::session_lifecycle::SessionGeneration;

use super::{
    button_bg_interaction::ButtonBgInteractiveComponent, consts::NORMAL_BUTTON,
    ui_interaction_system::NoPointerCapture, ui_setup::FontsResource,
};

#[derive(Debug, Component)]
pub struct MatchBannerRoot;

#[derive(Debug, Component)]
pub struct MatchBannerText;

#[derive(Debug, Component)]
pub struct MatchBannerDismiss;

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
            FocusPolicy::Pass,
            GlobalZIndex(20),
            MatchBannerRoot,
            NoPointerCapture,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    max_width: Val::Percent(92.0),
                    padding: UiRect::axes(Val::Px(36.0), Val::Px(24.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(14.0),
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
                        font_size: 36.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TextLayout::new_with_justify(Justify::Center),
                    MatchBannerText,
                ));
                card.spawn((
                    Text::new("Esc for menu"),
                    TextFont {
                        font: fonts.font.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.84, 0.89, 0.93)),
                ));
                card.spawn((
                    Button,
                    MatchBannerDismiss,
                    ButtonBgInteractiveComponent,
                    Node {
                        padding: UiRect::axes(Val::Px(24.0), Val::Px(8.0)),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(NORMAL_BUTTON),
                ))
                .with_child((
                    Text::new("Dismiss"),
                    TextFont {
                        font: fonts.font.clone(),
                        font_size: 18.0,
                        ..default()
                    },
                    FocusPolicy::Pass,
                ));
            });
        });
}

pub fn update_match_banner_system(
    outcome: Option<Res<MatchOutcome>>,
    rules: Res<SessionRules>,
    generation: Option<Res<SessionGeneration>>,
    mut previous: Local<Option<(u64, MatchOutcome)>>,
    dismiss: Query<&Interaction, (With<MatchBannerDismiss>, Changed<Interaction>)>,
    mut roots: Query<&mut Visibility, With<MatchBannerRoot>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<MatchBannerText>>,
) {
    let presentation = outcome.as_deref().copied().unwrap_or_default();
    let result = (
        generation.map_or(0, |generation| generation.0),
        presentation,
    );
    if *previous == Some(result) {
        if dismiss
            .iter()
            .any(|interaction| *interaction == Interaction::Pressed)
        {
            for mut visibility in &mut roots {
                *visibility = Visibility::Hidden;
            }
        }
        return;
    }
    *previous = Some(result);
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
            MatchOutcome::Winner(winner) if rules.spectator() => swarm_presentation_color(winner),
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
