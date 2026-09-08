//! Paused session controls and scenario selection.

use crate::scenario_selection::{Scenario, ScenarioSelection};
use crate::session_lifecycle::StartScenario;
use bevy::{input::InputSystems, prelude::*, ui::UiSystems};

#[derive(Resource, Default)]
pub struct ScenarioMenu {
    pub open: bool,
    /// Closing the menu consumes the frame's input as well.
    pub blocks_world_input: bool,
    was_paused_before_open: Option<bool>,
}

impl ScenarioMenu {
    pub fn toggle(&mut self) {
        self.blocks_world_input = true;
        self.open = !self.open;
    }

    /// Finish the menu transition after a new scenario has been installed.
    pub fn finish_scenario_start(&mut self) {
        self.open = false;
        self.blocks_world_input = true;
        self.was_paused_before_open = None;
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuInputSet {
    Keyboard,
    Actions,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Select(Scenario),
    Start,
    Quit,
}
#[derive(Component)]
pub struct MenuRoot;
#[derive(Component)]
struct MenuStatus;
#[derive(Component)]
struct MenuError;

pub struct ScenarioMenuPlugin;
impl Plugin for ScenarioMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScenarioMenu>()
            .init_resource::<ScenarioSelection>()
            .add_message::<StartScenario>()
            .configure_sets(
                PreUpdate,
                (MenuInputSet::Keyboard, MenuInputSet::Actions)
                    .chain()
                    .after(InputSystems)
                    .after(UiSystems::Focus),
            )
            .add_systems(PreUpdate, menu_keyboard.in_set(MenuInputSet::Keyboard))
            .add_systems(
                PreUpdate,
                (menu_actions, synchronize_pause)
                    .chain()
                    .in_set(MenuInputSet::Actions),
            )
            .add_systems(Startup, setup_menu)
            .add_systems(Update, refresh_menu);
    }
}

fn menu_keyboard(keys: Res<ButtonInput<KeyCode>>, mut menu: ResMut<ScenarioMenu>) {
    menu.blocks_world_input = menu.open;
    if keys.just_pressed(KeyCode::Escape) {
        menu.toggle();
    }
    menu.blocks_world_input |= menu.open;
}

fn menu_actions(
    buttons: Query<(&Interaction, &MenuAction), Changed<Interaction>>,
    mut menu: ResMut<ScenarioMenu>,
    mut selection: ResMut<ScenarioSelection>,
    mut starts: MessageWriter<StartScenario>,
    mut exit: MessageWriter<AppExit>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if menu.open {
            match action {
                MenuAction::Select(scenario) => {
                    selection.select(*scenario);
                }
                MenuAction::Start => {
                    starts.write(StartScenario(selection.next_launch));
                }
                MenuAction::Quit => {
                    exit.write(AppExit::Success);
                }
            }
        }
        menu.blocks_world_input = true;
    }
}

fn synchronize_pause(mut menu: ResMut<ScenarioMenu>, mut time: ResMut<Time<Virtual>>) {
    if menu.open {
        if menu.was_paused_before_open.is_none() {
            menu.was_paused_before_open = Some(time.is_paused());
        }
        time.pause();
    } else if let Some(was_paused) = menu.was_paused_before_open.take() {
        if was_paused {
            time.pause();
        } else {
            time.unpause();
        }
    }
    // Input runs after the clock update, so discard this frame's already-computed delta.
    if menu.blocks_world_input {
        time.advance_by(std::time::Duration::ZERO);
    }
}

fn menu_text(value: &str, size: f32) -> impl Bundle {
    (
        Text::new(value),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.95, 1.0)),
    )
}
fn button(action: MenuAction) -> impl Bundle {
    (
        Button,
        action,
        Node {
            padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(Color::srgb(0.12, 0.17, 0.24)),
        BorderColor::all(Color::srgb(0.25, 0.34, 0.44)),
    )
}
fn setup_menu(mut commands: Commands) {
    commands
        .spawn((
            MenuRoot,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(100),
            BackgroundColor(Color::srgba(0.015, 0.025, 0.04, 0.85)),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(500.0),
                    max_width: Val::Percent(95.0),
                    padding: UiRect::all(Val::Px(28.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(16.0),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.045, 0.065, 0.095)),
            ))
            .with_children(|panel| {
                panel.spawn(menu_text("Paused", 32.0));
                panel.spawn((menu_text("", 19.0), MenuStatus));
                panel.spawn(menu_text(
                    "Choose a scenario, then start it immediately.",
                    16.0,
                ));
                for (scenario, description) in [
                    (Scenario::Standard, "Standard: Face an advancing opponent"),
                    (Scenario::Sandbox, "Sandbox: Open-ended, no opponent"),
                    (Scenario::AiBattle, "AI Battle: Watch two autonomous swarms"),
                ] {
                    panel
                        .spawn(button(MenuAction::Select(scenario)))
                        .with_children(|b| {
                            b.spawn(menu_text(description, 17.0));
                        });
                }
                panel.spawn((menu_text("", 15.0), MenuError));
                panel.spawn(button(MenuAction::Start)).with_children(|b| {
                    b.spawn(menu_text("Start selected scenario", 18.0));
                });
                panel.spawn(button(MenuAction::Quit)).with_children(|b| {
                    b.spawn(menu_text("Quit to desktop", 18.0));
                });
            });
        });
}
#[allow(clippy::type_complexity)]
fn refresh_menu(
    menu: Res<ScenarioMenu>,
    selection: Res<ScenarioSelection>,
    mut roots: Query<&mut Node, With<MenuRoot>>,
    mut status: Query<&mut Text, (With<MenuStatus>, Without<MenuError>)>,
    mut errors: Query<(&mut Text, &mut Node), (With<MenuError>, Without<MenuRoot>)>,
    mut buttons: Query<(
        &MenuAction,
        &Interaction,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for mut root in &mut roots {
        root.display = if menu.open {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut text in &mut status {
        text.0 = format!(
            "Current: {}\nSelected: {}",
            selection.current.label(),
            selection.next_launch.label()
        );
    }
    for (mut text, mut node) in &mut errors {
        text.0 = selection.error.clone().unwrap_or_default();
        node.display = if selection.error.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (action, interaction, mut background, mut border) in &mut buttons {
        let selected = *action == MenuAction::Select(selection.next_launch);
        border.set_all(if selected {
            Color::srgb(0.35, 0.8, 0.75)
        } else {
            Color::srgb(0.25, 0.34, 0.44)
        });
        background.0 = if *interaction == Interaction::Hovered {
            Color::srgb(0.2, 0.28, 0.36)
        } else {
            Color::srgb(0.12, 0.17, 0.24)
        };
    }
}
