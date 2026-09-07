use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    time::{TimePlugin, TimeUpdateStrategy},
};
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d},
    intent::{BrushSelection, IntentKind, brush_selection_keyboard_system},
    nanobot::MatchOutcome,
    scenario_selection::{Scenario, ScenarioSelection},
    ui::scenario_menu::{MenuAction, MenuRoot, ScenarioMenuPlugin},
};

struct SettingsDirectory(PathBuf);
impl SettingsDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "nano-menu-playtest-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("{error}"),
            }
        }
    }
    fn path(&self) -> PathBuf {
        self.0.join("scenario.json")
    }
}
impl Drop for SettingsDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Resource, Default)]
struct SimulatedTicks(u32);
fn menu_app(selection: ScenarioSelection) -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            20,
        )))
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(selection)
        .init_resource::<SimulatedTicks>()
        .add_plugins(ScenarioMenuPlugin)
        .add_systems(FixedUpdate, |mut ticks: ResMut<SimulatedTicks>| {
            ticks.0 += 1
        });
    app.update();
    app.update();
    assert!(app.world().resource::<SimulatedTicks>().0 > 0);
    app
}
fn press_escape(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset(KeyCode::Escape);
}
fn click(app: &mut App, action: MenuAction) {
    let button = app
        .world_mut()
        .query::<(Entity, &MenuAction)>()
        .iter(app.world())
        .find(|(_, value)| **value == action)
        .unwrap()
        .0;
    app.world_mut()
        .entity_mut(button)
        .insert(Interaction::Pressed);
    app.update();
    app.world_mut().entity_mut(button).insert(Interaction::None);
}
fn visible(app: &mut App) -> bool {
    app.world_mut()
        .query_filtered::<&Node, With<MenuRoot>>()
        .single(app.world())
        .unwrap()
        .display
        != Display::None
}

#[test]
fn escape_pauses_before_the_next_fixed_tick_and_both_resume_controls_work() {
    let mut app = menu_app(ScenarioSelection::default());
    let ticks = app.world().resource::<SimulatedTicks>().0;
    press_escape(&mut app);
    assert!(visible(&mut app));
    assert_eq!(
        app.world().resource::<SimulatedTicks>().0,
        ticks,
        "opening ESC must prevent even the first fixed tick"
    );
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(app.world().resource::<SimulatedTicks>().0, ticks);
    press_escape(&mut app);
    assert!(!visible(&mut app));
    app.update();
    assert!(app.world().resource::<SimulatedTicks>().0 > ticks);
    press_escape(&mut app);
    let ticks = app.world().resource::<SimulatedTicks>().0;
    click(&mut app, MenuAction::Resume);
    assert!(!visible(&mut app));
    app.update();
    assert!(app.world().resource::<SimulatedTicks>().0 > ticks);
}

#[test]
fn menu_preserves_an_existing_simulation_pause() {
    let mut app = menu_app(ScenarioSelection::default());
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    let ticks = app.world().resource::<SimulatedTicks>().0;
    app.update();
    press_escape(&mut app);
    click(&mut app, MenuAction::Resume);
    app.update();
    assert!(app.world().resource::<Time<Virtual>>().is_paused());
    assert_eq!(app.world().resource::<SimulatedTicks>().0, ticks);
}

#[test]
fn paused_world_controls_do_not_leak_into_resume() {
    let mut app = menu_app(ScenarioSelection::default());
    app.init_resource::<BrushSelection>()
        .add_message::<MouseWheel>()
        .add_plugins(Camera2dFlyPlugin)
        .add_systems(Update, brush_selection_keyboard_system);
    let camera = app
        .world_mut()
        .spawn((
            FlyCamera2d {
                velocity: Vec2::new(100.0, 0.0),
                ..default()
            },
            Transform::default(),
            CameraZoom2d {
                zoom_speed: 0.1,
                zoom_min_max: (1.0, 100.0),
                zoom: 10.0,
            },
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ))
        .id();
    press_escape(&mut app);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit3);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyD);
    app.world_mut().write_message(MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: 1.0,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Gather
    );
    assert_eq!(
        app.world()
            .entity(camera)
            .get::<Transform>()
            .unwrap()
            .translation,
        Vec3::ZERO
    );
    assert_eq!(
        app.world()
            .entity(camera)
            .get::<CameraZoom2d>()
            .unwrap()
            .zoom,
        10.0
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    click(&mut app, MenuAction::Resume);
    app.update();
    assert_eq!(
        app.world()
            .entity(camera)
            .get::<Transform>()
            .unwrap()
            .translation,
        Vec3::ZERO
    );
    assert_eq!(
        app.world()
            .entity(camera)
            .get::<CameraZoom2d>()
            .unwrap()
            .zoom,
        10.0
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyD);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit3);
    app.world_mut().write_message(MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: 1.0,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Defend
    );
    assert!(
        app.world()
            .entity(camera)
            .get::<Transform>()
            .unwrap()
            .translation
            .x
            > 0.0
    );
    assert!(
        app.world()
            .entity(camera)
            .get::<CameraZoom2d>()
            .unwrap()
            .zoom
            < 10.0
    );
}

#[test]
fn scenario_button_confirms_next_launch_only_and_quit_emits_app_exit() {
    let directory = SettingsDirectory::new();
    let mut app = menu_app(ScenarioSelection::load(directory.path()));
    press_escape(&mut app);
    click(&mut app, MenuAction::Select(Scenario::Sandbox));
    let labels = app
        .world_mut()
        .query::<&Text>()
        .iter(app.world())
        .map(|text| text.0.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(labels.contains("Current: Standard\nNext launch: Sandbox"));
    assert_eq!(
        app.world().resource::<ScenarioSelection>().current,
        Scenario::Standard
    );
    assert_eq!(
        ScenarioSelection::load(directory.path()).current,
        Scenario::Sandbox
    );
    click(&mut app, MenuAction::Quit);
    assert!(
        app.world()
            .resource::<Messages<AppExit>>()
            .iter_current_update_messages()
            .any(|exit| *exit == AppExit::Success)
    );
}

#[test]
fn completed_match_still_allows_menu_selection_and_quit_through_agent_buttons() {
    use top_down_2d_rts_prototype_nano_swarm::agent_control::{
        AgentCommand, AgentControlCorePlugin, AgentRequest, ProtocolButton, RequestId,
    };
    let directory = SettingsDirectory::new();
    let mut app = menu_app(ScenarioSelection::load(directory.path()));
    app.insert_resource(MatchOutcome::Defeat);
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);
    for (id, button) in [
        ProtocolButton::MenuOpen,
        ProtocolButton::MenuSandbox,
        ProtocolButton::MenuQuit,
    ]
    .into_iter()
    .enumerate()
    {
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(id as u64),
                command: AgentCommand::ButtonPress { button },
            })
            .unwrap();
        app.update();
        let response = response
            .try_recv()
            .expect("request must complete during app.update()");
        assert!(response.ok, "{:?}", response.error);
        assert!(visible(&mut app));
    }
    assert_eq!(
        ScenarioSelection::load(directory.path()).current,
        Scenario::Sandbox
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::Defeat
    );
    assert!(
        app.world()
            .resource::<Messages<AppExit>>()
            .iter_current_update_messages()
            .any(|exit| *exit == AppExit::Success)
    );
}
