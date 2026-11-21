use std::collections::HashSet;

use bevy::prelude::{
    BackgroundColor, BorderColor, Bundle, Button, Changed, Children, Color, Commands, Component,
    DetectChanges, Entity, Handle, Interaction, Local, Node, PositionType, Query, Res, ResMut,
    Text, TextColor, TextFont, Val, With, Without, World, default,
};
use bevy::ui::{
    AlignItems, BorderRadius, FlexDirection, FlexWrap, JustifyContent, RelativeCursorPosition,
    UiRect,
};

use crate::{
    intent::{BrushSelection, IntentKind},
    nanobot::{MatchOutcome, SwarmId, swarm_presentation_color},
    session::SessionRules,
    ui::ui_setup::FontsResource,
    zones::zone_color,
};

use super::consts::NORMAL_BUTTON;
use super::ui_interaction_system::NoPointerCapture;

/// Marker for the panel root entity. The click and highlight systems only
/// touch descendants of this root, so a stray button in another system
/// cannot drive [`BrushSelection`] or have its visuals overridden.
///
/// The root is also tagged with [`NoPointerCapture`] (see
/// [`setup_intent_layer_panel`]) because the panel spans the full
/// window width as a layout anchor; without the marker its
/// `RelativeCursorPosition` would mark the cursor as "over UI" for
/// the whole viewport, blocking world-brush input even when the
/// cursor is far from any visible button.
#[derive(Debug, Component)]
pub struct IntentLayerPanelRoot;

/// Marker for a single intent-layer button. Carries the [`IntentKind`] the
/// button selects when clicked.
#[derive(Debug, Component, Clone, Copy)]
pub struct IntentLayerButton {
    pub kind: IntentKind,
}

#[derive(Component)]
pub struct IntentBrushHint;

const ACTIVE_BORDER_THICKNESS: f32 = 3.0;
const INACTIVE_BORDER_THICKNESS: f32 = 1.0;
const BORDER_INACTIVE: Color = Color::srgb(0.30, 0.30, 0.30);
const BORDER_ACTIVE: Color = Color::WHITE;
const PANEL_BACKGROUND: Color = Color::srgba(0.03, 0.04, 0.05, 0.90);
const PANEL_GAP: f32 = 6.0;
const PANEL_FONT_SIZE: f32 = 14.0;
const ERASE_HINT: &str = "Right-click erases the selected layer";
const ENDED_HINT: &str = "Painting disabled · Match finished";

fn layer_button_style(is_active: bool, layer_color: Color) -> (BackgroundColor, BorderColor, f32) {
    if is_active {
        (
            BackgroundColor(layer_color),
            BorderColor::all(BORDER_ACTIVE),
            ACTIVE_BORDER_THICKNESS,
        )
    } else {
        (
            BackgroundColor(NORMAL_BUTTON),
            BorderColor::all(BORDER_INACTIVE),
            INACTIVE_BORDER_THICKNESS,
        )
    }
}

pub fn setup_intent_layer_panel(
    mut commands: Commands,
    fonts: Res<FontsResource>,
    rules: Res<SessionRules>,
) {
    let font = &fonts.font;

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(4.0),
                right: Val::Px(4.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                align_items: AlignItems::Center,
                ..default()
            },
            IntentLayerPanelRoot,
            NoPointerCapture,
            RelativeCursorPosition::default(),
        ))
        .with_children(|parent| {
            parent.spawn(legend_row()).with_children(|row| {
                row.spawn(panel_text(
                    "Swarm Intent",
                    font,
                    PANEL_FONT_SIZE,
                    Color::WHITE,
                ));
                for kind in IntentKind::ALL {
                    let label = if rules.spectator() {
                        kind_name(kind).to_owned()
                    } else {
                        layer_label(kind)
                    };
                    let mut chip = row.spawn((
                        Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(5.0),
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                            border: UiRect::all(Val::Px(INACTIVE_BORDER_THICKNESS)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(NORMAL_BUTTON),
                        BorderColor::all(BORDER_INACTIVE),
                    ));
                    if !rules.spectator() {
                        chip.insert((
                            Button,
                            IntentLayerButton { kind },
                            RelativeCursorPosition::default(),
                        ));
                    }
                    chip.with_children(|chip| {
                        chip.spawn((
                            Node {
                                width: Val::Px(9.0),
                                height: Val::Px(9.0),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(2.0)),
                                ..default()
                            },
                            BackgroundColor(zone_color(kind)),
                            BorderColor::all(Color::srgb(0.17, 0.18, 0.2)),
                        ));
                        chip.spawn(panel_text(label, font, PANEL_FONT_SIZE, Color::WHITE));
                    });
                }
            });

            if !rules.spectator() {
                parent.spawn(legend_row()).with_children(|row| {
                    for (label, color) in [
                        ("Solid · Your swarm", swarm_presentation_color(SwarmId(0))),
                        ("/ hatch · Opponent", swarm_presentation_color(SwarmId(1))),
                        ("Two rows · Both swarms", Color::WHITE),
                    ] {
                        row.spawn(panel_text(label, font, 13.0, color));
                    }
                    row.spawn((
                        panel_text(ERASE_HINT, font, 13.0, Color::srgb(0.76, 0.80, 0.85)),
                        IntentBrushHint,
                    ));
                });
            }
        });
}

fn legend_row() -> impl Bundle {
    (
        Node {
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(PANEL_GAP),
            padding: UiRect::axes(Val::Px(7.0), Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(PANEL_BACKGROUND),
        RelativeCursorPosition::default(),
    )
}

fn panel_text(
    text: impl Into<String>,
    font: &Handle<bevy::prelude::Font>,
    font_size: f32,
    color: Color,
) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font: font.clone(),
            font_size,
            ..default()
        },
        TextColor(color),
    )
}

/// Rebuild the scenario-dependent panel inside the persistent application shell.
pub(crate) fn reset_for_session(world: &mut World) {
    if !world.contains_resource::<FontsResource>() {
        return;
    }
    let roots = world
        .query_filtered::<Entity, With<IntentLayerPanelRoot>>()
        .iter(world)
        .collect::<Vec<_>>();
    for root in roots {
        if let Ok(entity) = world.get_entity_mut(root) {
            entity.despawn();
        }
    }
    world.flush();
    world
        .run_system_cached(setup_intent_layer_panel)
        .expect("intent layer panel setup has its required resources");
    world.flush();
}

fn layer_label(kind: IntentKind) -> String {
    format!("{}: {}", layer_key_label(kind), kind_name(kind))
}

fn layer_key_label(kind: IntentKind) -> &'static str {
    match kind {
        IntentKind::Gather => "1",
        IntentKind::Build => "2",
        IntentKind::Defend => "3",
        IntentKind::Corridor => "4",
    }
}

fn kind_name(kind: IntentKind) -> &'static str {
    match kind {
        IntentKind::Gather => "Gather",
        IntentKind::Build => "Build",
        IntentKind::Defend => "Defend",
        IntentKind::Corridor => "Corridor",
    }
}

/// On `Interaction::Pressed` for a layer button, update [`BrushSelection`]
/// to the button's kind. Only buttons that are descendants of the panel
/// root can drive the selection.
#[allow(clippy::type_complexity)]
pub fn intent_layer_button_click_system(
    mut brush_selection: ResMut<BrushSelection>,
    panel_root_query: Query<Entity, With<IntentLayerPanelRoot>>,
    buttons: Query<
        (Entity, &Interaction, &IntentLayerButton),
        (Changed<Interaction>, With<Button>),
    >,
    children_query: Query<&Children>,
    menu: Option<Res<crate::ui::scenario_menu::ScenarioMenu>>,
    outcome: Option<Res<MatchOutcome>>,
) {
    if menu.is_some_and(|menu| menu.blocks_world_input)
        || outcome.is_some_and(|outcome| *outcome != MatchOutcome::InProgress)
    {
        return;
    }
    let Ok(panel_root) = panel_root_query.single() else {
        return;
    };
    let panel_children: HashSet<Entity> = children_query
        .iter_descendants(panel_root)
        .chain(std::iter::once(panel_root))
        .collect();

    for (entity, interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if !panel_children.contains(&entity) {
            continue;
        }
        brush_selection.kind = button.kind;
    }
}

/// Update the visual style of every panel-owned layer button to reflect
/// the active [`BrushSelection`] and remove control cues once the match ends.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update_intent_layer_panel_highlight(
    brush_selection: Res<BrushSelection>,
    outcome: Option<Res<MatchOutcome>>,
    panel_root_query: Query<Entity, With<IntentLayerPanelRoot>>,
    children_query: Query<&Children>,
    mut buttons: Query<
        (
            Entity,
            &IntentLayerButton,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Node,
        ),
        With<Button>,
    >,
    mut labels: Query<(&mut Text, &mut TextColor), Without<IntentBrushHint>>,
    mut hint: Query<&mut Text, With<IntentBrushHint>>,
    mut was_enabled: Local<Option<bool>>,
) {
    let enabled = outcome.is_none_or(|outcome| *outcome == MatchOutcome::InProgress);
    let enabled_changed = *was_enabled != Some(enabled);
    if !brush_selection.is_changed() && !enabled_changed {
        return;
    }
    *was_enabled = Some(enabled);
    if enabled_changed && let Ok(mut hint) = hint.single_mut() {
        **hint = if enabled { ERASE_HINT } else { ENDED_HINT }.to_owned();
    }
    let Ok(panel_root) = panel_root_query.single() else {
        return;
    };
    let panel_children: HashSet<Entity> = children_query
        .iter_descendants(panel_root)
        .chain(std::iter::once(panel_root))
        .collect();

    for (entity, button, mut bg, mut border, mut node) in &mut buttons {
        if !panel_children.contains(&entity) {
            continue;
        }
        let active = enabled && button.kind == brush_selection.kind;
        let (new_bg, new_border, thickness) = layer_button_style(active, zone_color(button.kind));
        *bg = new_bg;
        *border = new_border;
        node.border = UiRect::all(Val::Px(thickness));
        for child in children_query.iter_descendants(entity) {
            if let Ok((mut label, mut color)) = labels.get_mut(child) {
                color.0 = if active { Color::BLACK } else { Color::WHITE };
                if enabled_changed {
                    **label = if enabled {
                        layer_label(button.kind)
                    } else {
                        kind_name(button.kind).to_owned()
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Pure-function tests cover the styling rules; the system tests
    //! cover the wiring (panel-membership filter, click -> selection).

    use super::*;

    #[test]
    fn layer_button_style_marks_active_button_with_layer_color_and_thick_border() {
        let color = zone_color(IntentKind::Gather);
        let (bg, border, thickness) = layer_button_style(true, color);
        assert_eq!(bg.0, color);
        assert_eq!(border.top, BORDER_ACTIVE);
        assert_eq!(border.right, BORDER_ACTIVE);
        assert_eq!(border.bottom, BORDER_ACTIVE);
        assert_eq!(border.left, BORDER_ACTIVE);
        assert_eq!(thickness, ACTIVE_BORDER_THICKNESS);
    }

    #[test]
    fn layer_button_style_resets_inactive_button_to_normal() {
        let (bg, border, thickness) = layer_button_style(false, zone_color(IntentKind::Defend));
        assert_eq!(bg.0, NORMAL_BUTTON);
        assert_eq!(border.top, BORDER_INACTIVE);
        assert_eq!(border.right, BORDER_INACTIVE);
        assert_eq!(border.bottom, BORDER_INACTIVE);
        assert_eq!(border.left, BORDER_INACTIVE);
        assert_eq!(thickness, INACTIVE_BORDER_THICKNESS);
    }

    fn spawn_button_in_panel(app: &mut bevy::prelude::App, root: Entity, kind: IntentKind) {
        let button = app
            .world_mut()
            .spawn((
                Button,
                IntentLayerButton { kind },
                BackgroundColor(NORMAL_BUTTON),
                BorderColor::all(BORDER_INACTIVE),
                Node::default(),
            ))
            .id();
        app.world_mut().entity_mut(root).add_child(button);
    }

    #[test]
    fn highlight_marks_active_button_with_layer_color_and_thick_border() {
        let mut app = bevy::prelude::App::new();
        app.insert_resource(BrushSelection::new(IntentKind::Gather));
        app.add_systems(bevy::prelude::Update, update_intent_layer_panel_highlight);

        let root = app.world_mut().spawn(IntentLayerPanelRoot).id();
        for kind in IntentKind::ALL {
            spawn_button_in_panel(&mut app, root, kind);
        }
        app.update();

        let world = app.world_mut();
        let mut q = world.query::<(&IntentLayerButton, &BackgroundColor, &BorderColor, &Node)>();
        let (_button, bg, border, node) = q
            .iter(world)
            .find(|(b, _, _, _)| b.kind == IntentKind::Gather)
            .expect("active button must exist");
        let (expected_bg, expected_border, expected_thickness) =
            layer_button_style(true, zone_color(IntentKind::Gather));
        assert_eq!(*bg, expected_bg);
        assert_eq!(*border, expected_border);
        assert_eq!(node.border.left, Val::Px(expected_thickness));
    }

    #[test]
    fn highlight_resets_inactive_buttons_to_normal_background() {
        let mut app = bevy::prelude::App::new();
        app.insert_resource(BrushSelection::new(IntentKind::Build));
        app.add_systems(bevy::prelude::Update, update_intent_layer_panel_highlight);

        let root = app.world_mut().spawn(IntentLayerPanelRoot).id();
        for kind in IntentKind::ALL {
            spawn_button_in_panel(&mut app, root, kind);
        }
        app.update();

        let world = app.world_mut();
        let mut q = world.query::<(&IntentLayerButton, &BackgroundColor, &BorderColor, &Node)>();
        for (button, bg, border, node) in q.iter(world) {
            if button.kind == IntentKind::Build {
                continue;
            }
            let (expected_bg, expected_border, expected_thickness) =
                layer_button_style(false, zone_color(button.kind));
            assert_eq!(
                *bg, expected_bg,
                "{:?} must keep the inactive background",
                button.kind
            );
            assert_eq!(*border, expected_border);
            assert_eq!(node.border.left, Val::Px(expected_thickness));
        }
    }

    #[test]
    fn highlight_ignores_buttons_outside_the_panel() {
        // A loose `IntentLayerButton` (not a descendant of the panel
        // root) must keep its initial styling after the highlight system
        // runs, so future systems that spawn `IntentLayerButton`s
        // outside the panel are not silently overridden.
        let mut app = bevy::prelude::App::new();
        app.insert_resource(BrushSelection::new(IntentKind::Gather));
        app.add_systems(bevy::prelude::Update, update_intent_layer_panel_highlight);

        let marker = Color::srgb(0.5, 0.5, 0.5);
        let loose = app
            .world_mut()
            .spawn((
                Button,
                IntentLayerButton {
                    kind: IntentKind::Corridor,
                },
                BackgroundColor(marker),
                BorderColor::all(BORDER_ACTIVE),
                Node::default(),
            ))
            .id();
        // No `IntentLayerPanelRoot` spawned, so the loose button is not
        // a panel descendant.
        app.update();

        let world = app.world();
        let bg = world.entity(loose).get::<BackgroundColor>().unwrap();
        let border = world.entity(loose).get::<BorderColor>().unwrap();
        assert_eq!(bg.0, marker, "loose button bg must be untouched");
        assert_eq!(
            border.top, BORDER_ACTIVE,
            "loose button border must be untouched"
        );
    }

    #[test]
    fn click_system_writes_pressed_button_kind_into_brush_selection() {
        let mut app = bevy::prelude::App::new();
        app.init_resource::<BrushSelection>();
        app.add_systems(bevy::prelude::Update, intent_layer_button_click_system);

        let root = app.world_mut().spawn(IntentLayerPanelRoot).id();
        let button = app
            .world_mut()
            .spawn((
                Button,
                IntentLayerButton {
                    kind: IntentKind::Defend,
                },
                Interaction::Pressed,
            ))
            .id();
        app.world_mut().entity_mut(root).add_child(button);

        app.update();

        assert_eq!(
            app.world().resource::<BrushSelection>().kind,
            IntentKind::Defend,
            "clicking the Defend button must select Defend"
        );
    }

    #[test]
    fn click_system_ignores_buttons_outside_the_panel() {
        let mut app = bevy::prelude::App::new();
        app.init_resource::<BrushSelection>();
        app.add_systems(bevy::prelude::Update, intent_layer_button_click_system);

        app.world_mut().spawn((
            Button,
            IntentLayerButton {
                kind: IntentKind::Corridor,
            },
            Interaction::Pressed,
        ));

        app.update();

        assert_eq!(
            app.world().resource::<BrushSelection>().kind,
            IntentKind::Gather,
            "loose button must not change the default selection"
        );
    }
}
