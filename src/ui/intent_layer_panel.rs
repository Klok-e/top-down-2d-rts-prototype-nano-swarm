//! Visible swarm-intent layer controls.
//!
//! Keyboard number-row keys are the primary brush-layer binding; this panel
//! gives the player a visible control surface for the same action. It spawns
//! a row of buttons (one per [`IntentKind`]) that drive [`BrushSelection`]
//! and highlights the active button in the layer's zone-shader colour.

use std::collections::HashSet;

use bevy::prelude::{
    default, BackgroundColor, BorderColor, Bundle, Button, Changed, Children, Color, Commands,
    Component, DetectChanges, Entity, Interaction, Node, PositionType, Query, Res, ResMut, Text,
    TextColor, TextFont, Val, With,
};
use bevy::ui::{AlignItems, BorderRadius, FlexDirection, JustifyContent, UiRect};

use crate::intent::{BrushSelection, IntentKind};
use crate::ui::ui_setup::FontsResource;

use super::consts::NORMAL_BUTTON;

/// Marker for the panel root entity. The click and highlight systems only
/// touch descendants of this root, so a stray button in another system
/// cannot drive [`BrushSelection`] or have its visuals overridden.
#[derive(Debug, Component)]
pub struct IntentLayerPanelRoot;

/// Marker for a single intent-layer button. Carries the [`IntentKind`] the
/// button selects when clicked.
#[derive(Debug, Component, Clone, Copy)]
pub struct IntentLayerButton {
    pub kind: IntentKind,
}

/// Stable colour for each layer button. Matches the zone shader palette
/// (`zone_shader.wgsl`) so the on-screen UI cue lines up with the colour
/// the player sees in the world when they paint that layer.
const LAYER_COLORS: [(IntentKind, Color); IntentKind::COUNT] = [
    (IntentKind::Gather, Color::srgb(0.85, 0.20, 0.20)),
    (IntentKind::Build, Color::srgb(0.85, 0.20, 0.85)),
    (IntentKind::Defend, Color::srgb(0.20, 0.30, 0.90)),
    (IntentKind::Corridor, Color::srgb(0.85, 0.80, 0.10)),
];

const ACTIVE_BORDER_THICKNESS: f32 = 3.0;
const INACTIVE_BORDER_THICKNESS: f32 = 1.0;
const BORDER_INACTIVE: Color = Color::srgb(0.30, 0.30, 0.30);
const BORDER_ACTIVE: Color = Color::srgb(1.0, 1.0, 1.0);
const PANEL_GAP: f32 = 8.0;
const BUTTON_PADDING_X: f32 = 12.0;
const BUTTON_PADDING_Y: f32 = 6.0;
const PANEL_FONT_SIZE: f32 = 16.0;
const PANEL_TOP: f32 = 8.0;
const LABEL_GAP: f32 = 6.0;

fn layer_color(kind: IntentKind) -> Color {
    LAYER_COLORS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, c)| *c)
        .expect("every IntentKind has a layer color")
}

/// Visual style for a layer button: background, border, and border
/// thickness. Pure function so unit tests can verify the styling without a
/// Bevy `App`.
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

/// Spawn the intent-layer panel: a horizontal row of four buttons across
/// the top-center of the screen, plus a small "Swarm Intent" label so the
/// player knows what the controls affect. The parent spans the full width
/// and centers its children, so the row stays centered as more buttons
/// are added.
pub fn setup_intent_layer_panel(mut commands: Commands, fonts: Res<FontsResource>) {
    let font = fonts.font.clone();

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(PANEL_TOP),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(PANEL_GAP),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            IntentLayerPanelRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Swarm Intent"),
                TextFont {
                    font: font.clone(),
                    font_size: PANEL_FONT_SIZE,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                Node {
                    margin: UiRect {
                        right: Val::Px(LABEL_GAP),
                        ..default()
                    },
                    ..default()
                },
            ));
            for (kind, _) in LAYER_COLORS {
                parent
                    .spawn(spawn_layer_button(kind))
                    .with_children(|button| {
                        button.spawn((
                            Text::new(layer_label(kind)),
                            TextFont {
                                font: font.clone(),
                                font_size: PANEL_FONT_SIZE,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                    });
            }
        });
}

fn spawn_layer_button(kind: IntentKind) -> impl Bundle {
    (
        Button,
        IntentLayerButton { kind },
        BackgroundColor(NORMAL_BUTTON),
        BorderColor::all(BORDER_INACTIVE),
        Node {
            padding: UiRect {
                left: Val::Px(BUTTON_PADDING_X),
                right: Val::Px(BUTTON_PADDING_X),
                top: Val::Px(BUTTON_PADDING_Y),
                bottom: Val::Px(BUTTON_PADDING_Y),
            },
            border: UiRect::all(Val::Px(INACTIVE_BORDER_THICKNESS)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
    )
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
) {
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
/// the active [`BrushSelection`]. Only runs when the selection changes
/// so the per-frame button-bg system can handle hover/press feedback
/// without being clobbered.
#[allow(clippy::type_complexity)]
pub fn update_intent_layer_panel_highlight(
    brush_selection: Res<BrushSelection>,
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
) {
    if !brush_selection.is_changed() {
        return;
    }
    let Ok(panel_root) = panel_root_query.single() else {
        return;
    };
    let panel_children: HashSet<Entity> = children_query
        .iter_descendants(panel_root)
        .chain(std::iter::once(panel_root))
        .collect();

    let active = brush_selection.kind;
    for (entity, button, mut bg, mut border, mut node) in &mut buttons {
        if !panel_children.contains(&entity) {
            continue;
        }
        let (new_bg, new_border, thickness) =
            layer_button_style(button.kind == active, layer_color(button.kind));
        *bg = new_bg;
        *border = new_border;
        node.border = UiRect::all(Val::Px(thickness));
    }
}

#[cfg(test)]
mod tests {
    //! Pure-function tests cover the styling rules; the system tests
    //! cover the wiring (panel-membership filter, click -> selection).

    use super::*;

    #[test]
    fn layer_button_style_marks_active_button_with_layer_color_and_thick_border() {
        let color = layer_color(IntentKind::Gather);
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
        let (bg, border, thickness) = layer_button_style(false, layer_color(IntentKind::Defend));
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
            layer_button_style(true, layer_color(IntentKind::Gather));
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
                layer_button_style(false, layer_color(button.kind));
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
