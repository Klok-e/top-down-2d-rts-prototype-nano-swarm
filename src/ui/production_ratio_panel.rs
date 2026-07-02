//! Right-side Production Ratio UI (issue #32).
//!
//! Three independent `+` / `-` sliders (Worker / Hauler /
//! Defender) on a panel that is always visible during
//! gameplay. Clicks mutate the global [`ProductionRatio`]
//! resource; the displayed percentages are `weight /
//! total_weight * 100`, rounded. The "total cannot become
//! zero" invariant is enforced by
//! [`ProductionRatio::try_change_weight`], so a rejected
//! change is silent in the UI.

use bevy::prelude::*;
use bevy::ui::{
    AlignItems, BorderRadius, FlexDirection, JustifyContent, PositionType, RelativeCursorPosition,
    UiRect, Val,
};

use crate::nanobot::{NanobotType, ProductionRatio};

use super::ui_setup::FontsResource;

/// Marker for the panel root entity. Click and update
/// systems only touch descendants of this root, so a stray
/// button in another system cannot drive the global
/// [`ProductionRatio`].
#[derive(Debug, Component)]
pub struct ProductionRatioPanelRoot;

#[derive(Debug, Component, Clone, Copy, PartialEq, Eq)]
pub enum SliderDirection {
    Increase,
    Decrease,
}

/// Marker for a single `+` / `-` button.
#[derive(Debug, Component, Clone, Copy)]
pub struct ProductionRatioSlider {
    pub kind: NanobotType,
    pub direction: SliderDirection,
}

/// Marker for the value-text label of one row.
#[derive(Debug, Component, Clone, Copy)]
pub struct ProductionRatioValueText {
    pub kind: NanobotType,
}

/// Slider step in weight units. Issue #32 acceptance:
/// "Slider step size is 5."
pub const SLIDER_STEP: i32 = 5;

/// Panel layout constants. Public so the layout can be
/// asserted from tests / screenshot scripts.
pub const PANEL_TOP: f32 = 8.0;
pub const PANEL_RIGHT: f32 = 8.0;
pub const PANEL_WIDTH: f32 = 220.0;
pub const PANEL_PADDING: f32 = 10.0;
pub const PANEL_FONT_SIZE: f32 = 16.0;
pub const PANEL_TITLE_FONT_SIZE: f32 = 18.0;
pub const PANEL_GAP: f32 = 6.0;
pub const BUTTON_PADDING_X: f32 = 10.0;
pub const BUTTON_PADDING_Y: f32 = 4.0;

fn type_label_color(kind: NanobotType) -> Color {
    match kind {
        NanobotType::Worker => Color::srgb(0.85, 0.65, 0.30),
        NanobotType::Hauler => Color::srgb(0.30, 0.75, 0.85),
        NanobotType::Defender => Color::srgb(0.40, 0.55, 0.95),
    }
}

fn type_label(kind: NanobotType) -> &'static str {
    match kind {
        NanobotType::Worker => "Worker",
        NanobotType::Hauler => "Hauler",
        NanobotType::Defender => "Defender",
    }
}

fn row_node() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(PANEL_GAP),
        ..default()
    }
}

fn slider_button_node() -> Node {
    Node {
        padding: UiRect {
            left: Val::Px(BUTTON_PADDING_X),
            right: Val::Px(BUTTON_PADDING_X),
            top: Val::Px(BUTTON_PADDING_Y),
            bottom: Val::Px(BUTTON_PADDING_Y),
        },
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(4.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        min_width: Val::Px(24.0),
        ..default()
    }
}

fn slider_button_bundle(
    slider: ProductionRatioSlider,
    glyph: &str,
    font: Handle<Font>,
) -> impl Bundle {
    (
        Button,
        slider,
        BackgroundColor(Color::srgb(0.20, 0.20, 0.22)),
        BorderColor::all(Color::srgb(0.30, 0.30, 0.30)),
        // `check_ui_interaction` (the brush's UI-capture gate)
        // queries `RelativeCursorPosition`; without it a slider
        // button under the cursor is invisible to that system, so
        // `is_pointer_over_ui` stays false and the world brush
        // paints through the panel on a `+` / `-` click. Mirrors
        // the intent-layer button wiring.
        RelativeCursorPosition::default(),
        slider_button_node(),
        Text::new(glyph),
        TextFont {
            font,
            font_size: PANEL_FONT_SIZE,
            ..default()
        },
        TextColor(Color::WHITE),
    )
}

fn value_text_bundle(kind: NanobotType, percent: u32, font: Handle<Font>) -> impl Bundle {
    (
        Text::new(format!("{percent}%")),
        TextFont {
            font,
            font_size: PANEL_FONT_SIZE,
            ..default()
        },
        TextColor(Color::WHITE),
        ProductionRatioValueText { kind },
        Node {
            min_width: Val::Px(48.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
    )
}

/// Spawn the right-side Production Ratio panel. Always
/// visible during gameplay, anchored top-right so it does
/// not conflict with the top intent-layer panel or the
/// left status panel.
pub fn setup_production_ratio_panel(mut commands: Commands, fonts: Res<FontsResource>) {
    let font = fonts.font.clone();
    let initial = ProductionRatio::default();

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(PANEL_TOP),
                right: Val::Px(PANEL_RIGHT),
                width: Val::Px(PANEL_WIDTH),
                padding: UiRect::all(Val::Px(PANEL_PADDING)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(PANEL_GAP),
                align_items: AlignItems::Stretch,
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.05, 0.78)),
            ProductionRatioPanelRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Production Ratio"),
                TextFont {
                    font: font.clone(),
                    font_size: PANEL_TITLE_FONT_SIZE,
                    ..default()
                },
                TextColor(Color::WHITE),
                Node {
                    margin: UiRect::bottom(Val::Px(2.0)),
                    ..default()
                },
            ));
            // Order is the stable glossary order so the
            // panel always reads Worker / Hauler / Defender
            // top to bottom.
            for kind in NanobotType::ALL {
                parent.spawn(row_node()).with_children(|row| {
                    row.spawn((
                        Text::new(type_label(kind)),
                        TextFont {
                            font: font.clone(),
                            font_size: PANEL_FONT_SIZE,
                            ..default()
                        },
                        TextColor(type_label_color(kind)),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    row.spawn(slider_button_bundle(
                        ProductionRatioSlider {
                            kind,
                            direction: SliderDirection::Decrease,
                        },
                        "-",
                        font.clone(),
                    ));
                    row.spawn(value_text_bundle(
                        kind,
                        initial.percentage(kind),
                        font.clone(),
                    ));
                    row.spawn(slider_button_bundle(
                        ProductionRatioSlider {
                            kind,
                            direction: SliderDirection::Increase,
                        },
                        "+",
                        font.clone(),
                    ));
                });
            }
        });
}

/// On `Interaction::Pressed` for a slider button, mutate
/// the global [`ProductionRatio`] by [`SLIDER_STEP`] in the
/// button's direction. Only buttons that are descendants
/// of the panel root can drive the ratio.
///
/// Uses the same `Changed<Interaction>` filter as the
/// layer-panel click system, so each press drives one
/// change and a held button does not auto-repeat.
#[allow(clippy::type_complexity)]
pub fn production_ratio_slider_click_system(
    mut ratio: ResMut<ProductionRatio>,
    panel_root_query: Query<Entity, With<ProductionRatioPanelRoot>>,
    children_query: Query<&Children>,
    buttons: Query<
        (Entity, &Interaction, &ProductionRatioSlider),
        (Changed<Interaction>, With<Button>),
    >,
) {
    let Ok(panel_root) = panel_root_query.single() else {
        return;
    };
    let panel_descendants: std::collections::HashSet<Entity> =
        children_query.iter_descendants(panel_root).collect();

    for (entity, interaction, slider) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if !panel_descendants.contains(&entity) {
            continue;
        }
        let delta = match slider.direction {
            SliderDirection::Increase => SLIDER_STEP,
            SliderDirection::Decrease => -SLIDER_STEP,
        };
        ratio.try_change_weight(slider.kind, delta);
    }
}

/// Refresh the percentage labels whenever [`ProductionRatio`]
/// changes. The `is_changed` gate means the system is a
/// no-op on ticks where the player did not move a slider.
pub fn update_production_ratio_value_texts(
    ratio: Res<ProductionRatio>,
    mut value_texts: Query<(&ProductionRatioValueText, &mut Text)>,
) {
    if !ratio.is_changed() {
        return;
    }
    for (marker, mut text) in &mut value_texts {
        *text = Text::new(format!("{}%", ratio.percentage(marker.kind)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_ratio_percentage_default_60_30_10() {
        let r = ProductionRatio::default();
        assert_eq!(r.percentage(NanobotType::Worker), 60);
        assert_eq!(r.percentage(NanobotType::Hauler), 30);
        assert_eq!(r.percentage(NanobotType::Defender), 10);
    }

    #[test]
    fn production_ratio_percentage_arbitrary_weights() {
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 7);
        r.set_weight(NanobotType::Hauler, 3);
        assert_eq!(r.percentage(NanobotType::Worker), 70);
        assert_eq!(r.percentage(NanobotType::Hauler), 30);
        assert_eq!(r.percentage(NanobotType::Defender), 0);
    }

    #[test]
    fn production_ratio_percentage_empty_is_all_zero() {
        let r = ProductionRatio::new();
        assert_eq!(r.percentage(NanobotType::Worker), 0);
        assert_eq!(r.percentage(NanobotType::Hauler), 0);
        assert_eq!(r.percentage(NanobotType::Defender), 0);
    }

    #[test]
    fn production_ratio_percentage_rounds_to_nearest_integer() {
        // 1/1/1 -> 33.33%, rounds to 33.
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 1);
        r.set_weight(NanobotType::Hauler, 1);
        r.set_weight(NanobotType::Defender, 1);
        assert_eq!(r.percentage(NanobotType::Worker), 33);
        assert_eq!(r.percentage(NanobotType::Hauler), 33);
        assert_eq!(r.percentage(NanobotType::Defender), 33);
    }

    #[test]
    fn production_ratio_percentage_dropping_a_type_zeros_its_share() {
        // 6/3/0 -> 67/33/0.
        let mut r = ProductionRatio::new();
        r.set_weight(NanobotType::Worker, 6);
        r.set_weight(NanobotType::Hauler, 3);
        assert_eq!(r.percentage(NanobotType::Worker), 67);
        assert_eq!(r.percentage(NanobotType::Hauler), 33);
        assert_eq!(r.percentage(NanobotType::Defender), 0);
    }
}
