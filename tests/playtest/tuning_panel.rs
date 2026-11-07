use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use bevy::{
    camera::RenderTargetInfo,
    input::{
        ButtonState, InputPlugin,
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    time::{TimePlugin, TimeUpdateStrategy},
    ui::RelativeCursorPosition,
};
use top_down_2d_rts_prototype_nano_swarm::{
    fly_camera::{Camera2dFlyPlugin, CameraZoom2d, FlyCamera2d},
    intent::{BrushSelection, IntentGrid, IntentKind, brush_selection_keyboard_system},
    nanobot::SwarmId,
    reachability::SwarmReachabilityMap,
    session::SessionRules,
    tuning::{Tuning, TuningRevision, TuningStore},
    ui::{
        UiHandling, check_ui_interaction,
        scenario_menu::{ScenarioMenu, ScenarioMenuPlugin},
        tuning_panel::{
            TuningEditor, TuningGroupButton, TuningPanelPlugin, TuningPanelRoot, TuningValueButton,
        },
    },
    zones::zone_brush_system,
};

struct StoreDirectory(PathBuf);
impl StoreDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp")
            .join(format!(
                "tuning-panel-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("balance.ron")
    }
}
impl Drop for StoreDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Resource, Default)]
struct SimulatedTicks(u32);
fn editor_app(directory: &StoreDirectory) -> App {
    let store = TuningStore::new(directory.path());
    store.save(&Tuning::default()).unwrap();
    let (store, tuning) = TuningStore::load(directory.path()).unwrap();
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        InputPlugin,
        TuningPanelPlugin,
        ScenarioMenuPlugin,
        Camera2dFlyPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        20,
    )))
    .insert_resource(store)
    .insert_resource(tuning)
    .init_resource::<TuningRevision>()
    .init_resource::<BrushSelection>()
    .init_resource::<UiHandling>()
    .init_resource::<SimulatedTicks>()
    .add_systems(
        Update,
        (brush_selection_keyboard_system, check_ui_interaction).chain(),
    )
    .add_systems(FixedUpdate, |mut ticks: ResMut<SimulatedTicks>| {
        ticks.0 += 1
    });
    app.update();
    app.update();
    app
}
fn key(app: &mut App, code: KeyCode, text: Option<&str>) {
    app.world_mut().write_message(KeyboardInput {
        key_code: code,
        logical_key: text.map_or(
            Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
            |text| Key::Character(text.into()),
        ),
        state: ButtonState::Pressed,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    app.world_mut().write_message(KeyboardInput {
        key_code: code,
        logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
        state: ButtonState::Released,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}
fn click_group(app: &mut App, group: &str) {
    let entity = app
        .world_mut()
        .query::<(Entity, &TuningGroupButton)>()
        .iter(app.world())
        .find(|(_, button)| button.0 == group)
        .unwrap()
        .0;
    click(app, entity);
}
fn click_field(app: &mut App, key: &str) {
    let entity = app
        .world_mut()
        .query::<(Entity, &TuningValueButton)>()
        .iter(app.world())
        .find(|(_, button)| button.0 == key)
        .unwrap()
        .0;
    click(app, entity);
}
fn click(app: &mut App, entity: Entity) {
    app.world_mut()
        .entity_mut(entity)
        .insert(Interaction::Pressed);
    app.update();
    app.world_mut().entity_mut(entity).insert(Interaction::None);
    app.update();
}
fn live(app: &App, key: &str) -> f64 {
    app.world().resource::<Tuning>().get(key).unwrap()
}
fn saved(directory: &StoreDirectory, key: &str) -> f64 {
    TuningStore::load(directory.path())
        .unwrap()
        .1
        .get(key)
        .unwrap()
}

#[test]
fn tuning_panel_commits_each_valid_text_event_and_steps_seconds_without_pausing() {
    let directory = StoreDirectory::new();
    let mut app = editor_app(&directory);
    key(&mut app, KeyCode::F2, None);
    let ticks = app.world().resource::<SimulatedTicks>().0;
    click_group(&mut app, "production");
    click_field(&mut app, "production.cost_per_bot");
    key(&mut app, KeyCode::Digit5, Some("5"));
    assert_eq!(live(&app, "production.cost_per_bot"), 5.0);
    assert_eq!(saved(&directory, "production.cost_per_bot"), 5.0);
    key(&mut app, KeyCode::Digit2, Some("2"));
    assert_eq!(live(&app, "production.cost_per_bot"), 52.0);
    assert_eq!(saved(&directory, "production.cost_per_bot"), 52.0);
    assert_eq!(
        app.world().resource::<BrushSelection>().kind,
        IntentKind::Gather,
        "typing a number must not select an intent"
    );
    key(&mut app, KeyCode::ArrowUp, None);
    assert_eq!(saved(&directory, "production.cost_per_bot"), 53.0);
    click_field(&mut app, "production.ticks_per_bot");
    key(&mut app, KeyCode::ArrowUp, None);
    assert!(
        (saved(&directory, "production.ticks_per_bot") - 601.0 / 60.0).abs() < 1e-12,
        "seconds stepping must preserve one native simulation tick"
    );
    assert!(app.world().resource::<SimulatedTicks>().0 > ticks);
    assert!(!app.world().resource::<Time<Virtual>>().is_paused());
}

#[test]
fn tuning_panel_retains_last_accepted_value_after_invalid_text_and_save_failure() {
    let directory = StoreDirectory::new();
    let mut app = editor_app(&directory);
    key(&mut app, KeyCode::F2, None);
    click_group(&mut app, "production");
    click_field(&mut app, "production.cost_per_bot");
    key(&mut app, KeyCode::Digit5, Some("5"));
    let accepted_file = std::fs::read(directory.path()).unwrap();
    let revision = app.world().resource::<TuningRevision>().revision;
    click_field(&mut app, "production.cost_per_bot");
    key(&mut app, KeyCode::Minus, Some("-"));
    assert_eq!(live(&app, "production.cost_per_bot"), 5.0);
    assert_eq!(std::fs::read(directory.path()).unwrap(), accepted_file);
    assert!(
        app.world()
            .resource::<TuningEditor>()
            .errors
            .contains_key("production.cost_per_bot")
    );
    key(&mut app, KeyCode::Backspace, None);
    key(&mut app, KeyCode::Digit6, Some("6"));
    assert_eq!(live(&app, "production.cost_per_bot"), 6.0);
    assert!(
        !app.world()
            .resource::<TuningEditor>()
            .errors
            .contains_key("production.cost_per_bot")
    );
    let accepted_file = std::fs::read(directory.path()).unwrap();
    let revision = revision + 1;
    let blocked_target = directory.0.join("directory-target");
    std::fs::create_dir(&blocked_target).unwrap();
    app.insert_resource(TuningStore::new(blocked_target));
    click_field(&mut app, "production.cost_per_bot");
    key(&mut app, KeyCode::Digit7, Some("7"));
    assert_eq!(
        live(&app, "production.cost_per_bot"),
        6.0,
        "failed persistence must not publish a live edit"
    );
    assert_eq!(app.world().resource::<TuningRevision>().revision, revision);
    assert_eq!(std::fs::read(directory.path()).unwrap(), accepted_file);
    assert!(
        app.world()
            .resource::<TuningEditor>()
            .errors
            .contains_key("production.cost_per_bot")
    );
}

#[test]
fn tuning_panel_captures_world_input_and_escape_releases_focus_before_menu() {
    let directory = StoreDirectory::new();
    let mut app = editor_app(&directory);
    let grid = IntentGrid::new(16, 16);
    let reachability =
        SwarmReachabilityMap::new(&grid, [], [(SwarmId::PLAYER, Vec2::ZERO)]).unwrap();
    app.insert_resource(grid)
        .insert_resource(reachability)
        .init_resource::<SessionRules>()
        .add_systems(Update, zone_brush_system.after(check_ui_interaction));
    let mut window = Window {
        resolution: (1280, 720).into(),
        ..default()
    };
    window.set_cursor_position(Some(Vec2::new(640.0, 360.0)));
    app.world_mut().spawn(window);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Transform::default(),
            GlobalTransform::default(),
            FlyCamera2d::default(),
            CameraZoom2d {
                zoom: 1.0,
                zoom_speed: 0.1,
                zoom_min_max: (0.1, 20.0),
            },
        ))
        .id();
    app.world_mut()
        .entity_mut(camera)
        .get_mut::<Camera>()
        .unwrap()
        .computed
        .target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1280, 720),
        scale_factor: 1.0,
    });
    key(&mut app, KeyCode::F2, None);
    let root = app
        .world_mut()
        .query_filtered::<Entity, With<TuningPanelRoot>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(root)
        .insert(RelativeCursorPosition {
            cursor_over: true,
            normalized: Some(Vec2::splat(0.5)),
        });
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut().write_message(MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: 2.0,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    assert_eq!(
        app.world()
            .resource::<IntentGrid>()
            .iter_active_cells()
            .count(),
        0,
        "clicking the panel must not paint the world"
    );
    assert_eq!(
        app.world()
            .entity(camera)
            .get::<CameraZoom2d>()
            .unwrap()
            .zoom,
        1.0,
        "scrolling the panel must not zoom the camera"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset(MouseButton::Left);
    click_group(&mut app, "production");
    click_field(&mut app, "production.cost_per_bot");
    let position = app
        .world()
        .entity(camera)
        .get::<Transform>()
        .unwrap()
        .translation;
    key(&mut app, KeyCode::KeyD, Some("d"));
    assert_eq!(
        app.world()
            .entity(camera)
            .get::<Transform>()
            .unwrap()
            .translation,
        position,
        "focused text input must not pan the camera"
    );
    key(&mut app, KeyCode::Escape, None);
    assert!(app.world().resource::<TuningEditor>().focused.is_none());
    assert!(!app.world().resource::<ScenarioMenu>().open);
    key(&mut app, KeyCode::Escape, None);
    assert!(app.world().resource::<ScenarioMenu>().open);
}

#[test]
fn agent_tuning_acceptance_replaces_rejected_field_text_including_noop() {
    use top_down_2d_rts_prototype_nano_swarm::agent_control::{
        AgentCommand, AgentControlCorePlugin, AgentRequest, RequestId,
    };
    let directory = StoreDirectory::new();
    let mut app = editor_app(&directory);
    let (control, plugin) = AgentControlCorePlugin::channel(4);
    app.add_plugins(plugin);
    key(&mut app, KeyCode::F2, None);
    click_group(&mut app, "production");
    for id in 0..2 {
        click_field(&mut app, "production.cost_per_bot");
        key(&mut app, KeyCode::Minus, Some("-"));
        assert!(
            app.world()
                .resource::<TuningEditor>()
                .errors
                .contains_key("production.cost_per_bot")
        );
        let revision = app.world().resource::<TuningRevision>().revision;
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(id),
                command: AgentCommand::TuningSet {
                    key: "production.cost_per_bot".into(),
                    value: 12.0,
                },
            })
            .unwrap();
        app.update();
        let response = response.try_recv().unwrap();
        assert!(response.ok, "{:?}", response.error);
        assert!(
            !app.world()
                .resource::<TuningEditor>()
                .errors
                .contains_key("production.cost_per_bot"),
            "a successful external edit must clear the rejected UI value even when the accepted value is unchanged"
        );
        assert_eq!(saved(&directory, "production.cost_per_bot"), 12.0);
        if id == 1 {
            assert_eq!(app.world().resource::<TuningRevision>().revision, revision);
        }
        let button = app
            .world_mut()
            .query::<(Entity, &TuningValueButton)>()
            .iter(app.world())
            .find(|(_, button)| button.0 == "production.cost_per_bot")
            .unwrap()
            .0;
        let label = app.world().get::<Children>(button).unwrap()[0];
        assert_eq!(
            app.world().get::<Text>(label).unwrap().0.replace('|', ""),
            "12"
        );
        key(&mut app, KeyCode::Escape, None);
        key(&mut app, KeyCode::F2, None);
        key(&mut app, KeyCode::F2, None);
        assert_eq!(
            app.world().get::<Text>(label).unwrap().0,
            "12",
            "reopening must show the accepted external value"
        );
    }
}
