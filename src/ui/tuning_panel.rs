use std::collections::{BTreeMap, BTreeSet};

use bevy::{
    ecs::message::MessageCursor,
    input::{
        ButtonState, InputSystems,
        keyboard::KeyboardInput,
        mouse::{MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    ui::{RelativeCursorPosition, UiSystems},
};

use super::scenario_menu::{MenuInputSet, ScenarioMenu};
use crate::tuning::{Tuning, set_value};

#[derive(Resource, Default)]
pub struct TuningEditor {
    pub open: bool,
    pub focused: Option<&'static str>,
    pub expanded: BTreeSet<&'static str>,
    pub errors: BTreeMap<&'static str, String>,
    raw: BTreeMap<&'static str, String>,
    capture_keyboard: bool,
    capture_pointer: bool,
    select_all: bool,
    caret: usize,
}

impl TuningEditor {
    pub fn keyboard_captured(&self) -> bool {
        self.capture_keyboard || (self.open && self.focused.is_some())
    }

    pub fn pointer_captured(&self) -> bool {
        self.open && self.capture_pointer
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            self.capture_keyboard |= self.focused.is_some();
            self.focused = None;
            self.capture_pointer = false;
        }
    }

    pub fn set_group_expanded(&mut self, group: &'static str, expanded: bool) {
        if expanded {
            self.expanded.insert(group);
        } else {
            self.expanded.remove(group);
            if self.focused.is_some_and(|key| {
                Tuning::fields()
                    .iter()
                    .any(|field| field.key == key && field.group == group)
            }) {
                self.focused = None;
            }
        }
    }
}

pub fn toggle_editor(world: &mut World, group: Option<&str>) -> Result<bool, String> {
    if let Some(group) = group {
        let group = Tuning::fields()
            .iter()
            .find(|field| field.group == group)
            .map(|field| field.group)
            .ok_or_else(|| format!("unknown tuning group: {group}"))?;
        let mut editor = world.resource_mut::<TuningEditor>();
        let expanded = !editor.expanded.contains(group);
        editor.set_group_expanded(group, expanded);
        editor.set_open(true);
    } else {
        let mut editor = world.resource_mut::<TuningEditor>();
        let open = !editor.open;
        editor.set_open(open);
    }
    Ok(world.resource::<TuningEditor>().open)
}

pub fn edit_value(world: &mut World, key: &str, raw: &str) -> Result<u64, String> {
    let key = Tuning::fields()
        .iter()
        .find(|field| field.key == key)
        .map(|field| field.key)
        .ok_or_else(|| format!("unknown tuning field: {key}"))?;
    let result = raw
        .parse::<f64>()
        .map_err(|_| "Enter a number".to_string())
        .and_then(|value| set_value(world, key, value));
    if let Some(mut editor) = world.get_resource_mut::<TuningEditor>() {
        editor.raw.insert(key, raw.to_string());
        if editor.focused == Some(key) {
            editor.caret = editor.caret.min(raw.len());
            while !raw.is_char_boundary(editor.caret) {
                editor.caret -= 1;
            }
        }
        match &result {
            Ok(_) => {
                editor.errors.remove(key);
            }
            Err(error) => {
                editor.errors.insert(key, error.clone());
            }
        }
    }
    result
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TuningInputSet;

#[derive(Component)]
pub struct TuningPanelRoot;
#[derive(Component)]
pub struct TuningGroupButton(pub &'static str);
#[derive(Component)]
struct GroupLabel(&'static str);
#[derive(Component)]
struct GroupBody(&'static str);
#[derive(Component)]
pub struct TuningValueButton(pub &'static str);
#[derive(Component)]
struct ValueLabel(&'static str);
#[derive(Component)]
struct FieldError(&'static str);

pub struct TuningPanelPlugin;
impl Plugin for TuningPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TuningEditor>()
            .add_message::<KeyboardInput>()
            .add_message::<MouseWheel>()
            .configure_sets(
                PreUpdate,
                TuningInputSet
                    .after(InputSystems)
                    .after(UiSystems::Focus)
                    .before(MenuInputSet::Keyboard),
            )
            .add_systems(PreUpdate, tuning_input.in_set(TuningInputSet))
            .add_systems(Startup, setup_panel)
            .add_systems(Update, refresh_panel);
    }
}

fn text(value: impl Into<String>, size: f32) -> impl Bundle {
    (
        Text::new(value),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb(0.88, 0.92, 0.96)),
    )
}

fn group_label(group: &str) -> String {
    let mut characters = group.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

fn setup_panel(mut commands: Commands) {
    commands
        .spawn((
            TuningPanelRoot,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(430.0),
                max_width: Val::Percent(95.0),
                padding: UiRect::all(Val::Px(14.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            GlobalZIndex(90),
            ScrollPosition::default(),
            RelativeCursorPosition::default(),
            BackgroundColor(Color::srgba(0.035, 0.05, 0.072, 0.98)),
        ))
        .with_children(|panel| {
            let mut groups = BTreeSet::new();
            for field in Tuning::fields() {
                if !groups.insert(field.group) {
                    continue;
                }
                panel
                    .spawn((
                        Button,
                        TuningGroupButton(field.group),
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(10.0)),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.09, 0.125, 0.17)),
                    ))
                    .with_children(|button| {
                        button.spawn((
                            text(group_label(field.group), 17.0),
                            GroupLabel(field.group),
                        ));
                    });
                panel
                    .spawn((
                        GroupBody(field.group),
                        Node {
                            display: Display::None,
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ))
                    .with_children(|body| {
                        for field in Tuning::fields()
                            .iter()
                            .filter(|item| item.group == field.group)
                        {
                            body.spawn(Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(2.0),
                                flex_shrink: 0.0,
                                ..default()
                            })
                            .with_children(|row| {
                                row.spawn(Node {
                                    align_items: AlignItems::Center,
                                    column_gap: Val::Px(8.0),
                                    ..default()
                                })
                                .with_children(|line| {
                                    line.spawn((
                                        text(field.label, 14.0),
                                        Node {
                                            flex_grow: 1.0,
                                            flex_basis: Val::Px(0.0),
                                            ..default()
                                        },
                                    ));
                                    line.spawn((
                                        Button,
                                        TuningValueButton(field.key),
                                        Node {
                                            width: Val::Px(94.0),
                                            padding: UiRect::all(Val::Px(6.0)),
                                            border: UiRect::all(Val::Px(1.0)),
                                            overflow: Overflow::clip(),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgb(0.065, 0.09, 0.12)),
                                        BorderColor::all(Color::srgb(0.23, 0.30, 0.38)),
                                    ))
                                    .with_children(|button| {
                                        button.spawn((text("", 15.0), ValueLabel(field.key)));
                                    });
                                    line.spawn((
                                        text(field.unit, 12.0),
                                        Node {
                                            width: Val::Px(36.0),
                                            ..default()
                                        },
                                    ));
                                });
                                row.spawn((
                                    Text::new(""),
                                    TextFont {
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(1.0, 0.48, 0.40)),
                                    FieldError(field.key),
                                    Node {
                                        display: Display::None,
                                        ..default()
                                    },
                                ));
                            });
                        }
                    });
            }
        });
}

fn tuning_input(
    world: &mut World,
    mut keyboard: Local<MessageCursor<KeyboardInput>>,
    mut wheel: Local<MessageCursor<MouseWheel>>,
) {
    let events = keyboard
        .read(world.resource::<Messages<KeyboardInput>>())
        .cloned()
        .collect::<Vec<_>>();
    let scroll = wheel
        .read(world.resource::<Messages<MouseWheel>>())
        .map(|event| {
            event.y
                * if event.unit == MouseScrollUnit::Line {
                    28.0
                } else {
                    1.0
                }
        })
        .sum::<f32>();
    let keys = world.get_resource::<ButtonInput<KeyCode>>();
    let toggle = keys.is_some_and(|keys| keys.just_pressed(KeyCode::F2));
    let escape = keys.is_some_and(|keys| keys.just_pressed(KeyCode::Escape));
    let ctrl = keys.is_some_and(|keys| {
        keys.pressed(KeyCode::ControlLeft)
            || keys.pressed(KeyCode::ControlRight)
            || keys.pressed(KeyCode::SuperLeft)
            || keys.pressed(KeyCode::SuperRight)
    });
    {
        let mut editor = world.resource_mut::<TuningEditor>();
        editor.capture_keyboard = editor.open && editor.focused.is_some();
    }
    if toggle {
        let _ = toggle_editor(world, None);
    }
    let menu_open = world
        .get_resource::<ScenarioMenu>()
        .is_some_and(|menu| menu.open);
    if menu_open {
        let mut editor = world.resource_mut::<TuningEditor>();
        editor.focused = None;
        editor.capture_keyboard = false;
        editor.capture_pointer = false;
        return;
    }
    if !world.resource::<TuningEditor>().open {
        return;
    }
    let hovered = world
        .query_filtered::<&RelativeCursorPosition, With<TuningPanelRoot>>()
        .iter(world)
        .any(|cursor| cursor.cursor_over());
    world.resource_mut::<TuningEditor>().capture_pointer = hovered;
    if hovered && scroll != 0.0 {
        let mut query =
            world.query_filtered::<(&mut ScrollPosition, &ComputedNode), With<TuningPanelRoot>>();
        for (mut position, node) in query.iter_mut(world) {
            let max =
                (node.content_size().y - node.size().y).max(0.0) * node.inverse_scale_factor();
            position.0.y = (position.0.y - scroll).clamp(0.0, max);
        }
    }
    let actions = world
        .query_filtered::<(
            &Interaction,
            Option<&TuningGroupButton>,
            Option<&TuningValueButton>,
        ), Changed<Interaction>>()
        .iter(world)
        .filter(|(interaction, _, _)| **interaction == Interaction::Pressed)
        .map(|(_, group, field)| (group.map(|item| item.0), field.map(|item| item.0)))
        .collect::<Vec<_>>();
    let clicked_field = actions.iter().any(|(_, field)| field.is_some());
    for (group, field) in actions {
        if let Some(group) = group {
            let _ = toggle_editor(world, Some(group));
        }
        if let Some(key) = field {
            let value = world
                .resource::<Tuning>()
                .get(key)
                .expect("metadata key exists");
            let mut editor = world.resource_mut::<TuningEditor>();
            editor.focused = Some(key);
            if !editor.errors.contains_key(key) {
                editor.raw.insert(key, format_number(value));
            }
            let raw = editor
                .raw
                .entry(key)
                .or_insert_with(|| format_number(value));
            editor.caret = raw.len();
            editor.select_all = true;
            editor.capture_keyboard = true;
        }
    }
    if !clicked_field
        && world
            .get_resource::<ButtonInput<MouseButton>>()
            .is_some_and(|mouse| mouse.just_pressed(MouseButton::Left))
    {
        world.resource_mut::<TuningEditor>().focused = None;
    }
    if escape {
        world.resource_mut::<TuningEditor>().focused = None;
        return;
    }
    let Some(key) = world.resource::<TuningEditor>().focused else {
        return;
    };
    for event in events
        .into_iter()
        .filter(|event| event.state == ButtonState::Pressed)
    {
        if event.key_code == KeyCode::Escape || event.key_code == KeyCode::F2 {
            continue;
        }
        if matches!(event.key_code, KeyCode::ArrowUp | KeyCode::ArrowDown) {
            let field = Tuning::fields()
                .iter()
                .find(|field| field.key == key)
                .expect("metadata key exists");
            let value = world.resource::<Tuning>().get(key).unwrap();
            let value = value
                + field.step
                    * if event.key_code == KeyCode::ArrowUp {
                        1.0
                    } else {
                        -1.0
                    };
            let raw = value.to_string();
            let _ = edit_value(world, key, &raw);
            let mut editor = world.resource_mut::<TuningEditor>();
            editor.caret = raw.len();
            editor.select_all = true;
            continue;
        }
        let edited = {
            let mut editor = world.resource_mut::<TuningEditor>();
            if ctrl && event.key_code == KeyCode::KeyA {
                editor.select_all = true;
                continue;
            }
            if ctrl {
                continue;
            }
            let mut raw = editor.raw.get(key).cloned().unwrap_or_default();
            let mut caret = editor.caret.min(raw.len());
            let mut changed = false;
            match event.key_code {
                KeyCode::ArrowLeft => {
                    caret = if editor.select_all {
                        0
                    } else {
                        caret.saturating_sub(1)
                    };
                    editor.select_all = false;
                }
                KeyCode::ArrowRight => {
                    caret = if editor.select_all {
                        raw.len()
                    } else {
                        (caret + 1).min(raw.len())
                    };
                    editor.select_all = false;
                }
                KeyCode::Home => {
                    caret = 0;
                    editor.select_all = false;
                }
                KeyCode::End => {
                    caret = raw.len();
                    editor.select_all = false;
                }
                KeyCode::Backspace | KeyCode::Delete => {
                    if editor.select_all {
                        raw.clear();
                        caret = 0;
                    } else if event.key_code == KeyCode::Backspace && caret > 0 {
                        caret -= 1;
                        raw.remove(caret);
                    } else if event.key_code == KeyCode::Delete && caret < raw.len() {
                        raw.remove(caret);
                    }
                    editor.select_all = false;
                    changed = true;
                }
                _ => {
                    if let Some(text) = event.text.filter(|text| {
                        !text.is_empty()
                            && text.chars().all(|character| {
                                character.is_ascii_digit() || ".-+eE".contains(character)
                            })
                    }) {
                        if editor.select_all {
                            raw.clear();
                            caret = 0;
                        }
                        raw.insert_str(caret, &text);
                        caret += text.len();
                        editor.select_all = false;
                        changed = true;
                    }
                }
            }
            editor.caret = caret;
            changed.then_some(raw)
        };
        if let Some(raw) = edited {
            let _ = edit_value(world, key, &raw);
        }
    }
}

fn format_number(value: f64) -> String {
    if value != 0.0 && value.abs() < 0.000001 {
        return format!("{value:.4e}");
    }
    let value = format!("{value:.6}");
    value
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

#[allow(clippy::type_complexity)]
fn refresh_panel(
    editor: Res<TuningEditor>,
    tuning: Res<Tuning>,
    mut nodes: Query<(
        &mut Node,
        Option<&TuningPanelRoot>,
        Option<&GroupBody>,
        Option<&FieldError>,
    )>,
    mut labels: Query<(
        &mut Text,
        Option<&GroupLabel>,
        Option<&ValueLabel>,
        Option<&FieldError>,
    )>,
    mut fields: Query<(&TuningValueButton, &mut BorderColor, &mut BackgroundColor)>,
) {
    for (mut node, root, group, error) in &mut nodes {
        let visible = if root.is_some() {
            editor.open
        } else if let Some(group) = group {
            editor.expanded.contains(group.0)
        } else if let Some(error) = error {
            editor.errors.contains_key(error.0)
        } else {
            continue;
        };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (mut text, group, field, error) in &mut labels {
        if let Some(group) = group {
            text.0 = format!(
                "{}  {}",
                if editor.expanded.contains(group.0) {
                    "−"
                } else {
                    "+"
                },
                group_label(group.0)
            );
        } else if let Some(field) = field {
            let raw = if editor.focused == Some(field.0) || editor.errors.contains_key(field.0) {
                editor.raw.get(field.0).cloned()
            } else {
                None
            }
            .unwrap_or_else(|| format_number(tuning.get(field.0).unwrap()));
            text.0 = if editor.focused == Some(field.0) {
                if editor.select_all {
                    raw
                } else {
                    let mut raw = raw;
                    raw.insert(editor.caret.min(raw.len()), '|');
                    raw
                }
            } else {
                raw
            };
        } else if let Some(error) = error {
            text.0 = editor
                .errors
                .get(error.0)
                .map(|message| {
                    if let Some(details) = message.strip_prefix("Cannot save ") {
                        let reason = details
                            .split_once(": ")
                            .map_or(details, |(_, reason)| reason);
                        format!("Save failed: {reason}")
                    } else {
                        message.clone()
                    }
                })
                .unwrap_or_default();
        }
    }
    for (field, mut border, mut background) in &mut fields {
        background.0 = if editor.focused == Some(field.0) && editor.select_all {
            Color::srgb(0.15, 0.27, 0.39)
        } else {
            Color::srgb(0.065, 0.09, 0.12)
        };
        *border = BorderColor::all(if editor.errors.contains_key(field.0) {
            Color::srgb(0.9, 0.3, 0.25)
        } else if editor.focused == Some(field.0) {
            Color::srgb(0.4, 0.7, 1.0)
        } else {
            Color::srgb(0.23, 0.30, 0.38)
        });
    }
}
