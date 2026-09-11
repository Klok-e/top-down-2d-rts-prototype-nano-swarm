use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use bevy::{asset::AssetPlugin, input::InputPlugin, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    DEFAULT_CAMERA_ZOOM, MAP_HEIGHT, MAP_WIDTH, MainCamera,
    battle_experiment::{BattleExperimentConfig, PacingId},
    battle_statistics::{
        BattleRun, BattleStatisticsConfig, BattleStatisticsPlugin, BattleSummary, RunStatus,
    },
    building::ProcessingFacility,
    fly_camera::{CameraZoom2d, FlyCamera2d},
    gameplay_pacing::GameplayPacing,
    intent::{IntentGrid, IntentKind},
    nanobot::{
        MatchOutcome, Nanobot, OpponentSwarmIdAlloc, StrategicController,
        StrategicControllerPlugin, Swarm, SwarmEliminationPlugin, SwarmId,
    },
    resources::{ResourceDeposit, ResourceKind, ResourceLedger},
    scenario::{PLAYER_CELL, cell_origin},
    scenario_selection::{Scenario, ScenarioSelection},
    session::SessionRules,
    session_lifecycle::{SessionEntity, SessionGeneration, SessionLifecyclePlugin, StartScenario},
    ui::{
        FontsResource,
        intent_layer_panel::{IntentLayerButton, setup_intent_layer_panel},
        scenario_menu::{MenuAction, ScenarioMenu, ScenarioMenuPlugin},
    },
};

#[path = "../common/mod.rs"]
mod common;

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "nano-session-restart-playtest-{}-{}",
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

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Resource, Default)]
struct SimulatedTicks(u32);

fn session_app(statistics_root: &Path) -> (App, Entity) {
    let mut app = common::minimal_app();
    app.insert_resource(IntentGrid::new(MAP_WIDTH as i32, MAP_HEIGHT as i32))
        .insert_resource(ScenarioSelection::load(
            statistics_root.join("scenario.json"),
        ))
        .insert_resource(BattleStatisticsConfig {
            output_root: statistics_root.to_path_buf(),
            ..default()
        })
        .init_resource::<OpponentSwarmIdAlloc>()
        .init_resource::<SimulatedTicks>()
        .insert_resource(FontsResource { font: default() })
        .add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin::default())
        .init_asset::<Image>()
        .add_plugins(InputPlugin)
        .add_plugins(ScenarioMenuPlugin)
        .add_plugins(SessionLifecyclePlugin)
        .add_plugins(StrategicControllerPlugin)
        .add_plugins(SwarmEliminationPlugin)
        .add_plugins(BattleStatisticsPlugin)
        .add_systems(Startup, setup_intent_layer_panel)
        .add_systems(FixedUpdate, |mut ticks: ResMut<SimulatedTicks>| {
            ticks.0 += 1
        });

    let camera = app
        .world_mut()
        .spawn((
            MainCamera,
            Transform::from_translation(cell_origin(PLAYER_CELL).extend(0.0)),
            Projection::Orthographic(OrthographicProjection {
                scale: DEFAULT_CAMERA_ZOOM,
                ..OrthographicProjection::default_2d()
            }),
            CameraZoom2d {
                zoom_speed: 0.15,
                zoom_min_max: (1.0, 100.0),
                zoom: DEFAULT_CAMERA_ZOOM,
            },
            FlyCamera2d::default(),
        ))
        .id();
    app.update();
    (app, camera)
}

fn count<T: Component>(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<T>>()
        .iter(world)
        .count()
}

fn assert_scenario_shape(app: &mut App, scenario: Scenario) {
    let (swarms, nanobots, facilities, deposits, controllers) = match scenario {
        Scenario::Standard => (2, 18, 2, 6, 1),
        Scenario::Sandbox => (1, 9, 1, 6, 0),
        Scenario::AiBattle => (2, 18, 2, 6, 2),
    };
    assert_eq!(
        app.world().resource::<ScenarioSelection>().current,
        scenario
    );
    assert_eq!(
        app.world().resource::<SessionRules>().scenario_name,
        scenario.definition().rules.scenario_name
    );
    assert_eq!(count::<Swarm>(app.world_mut()), swarms);
    assert_eq!(count::<Nanobot>(app.world_mut()), nanobots);
    assert_eq!(count::<ProcessingFacility>(app.world_mut()), facilities);
    assert_eq!(count::<ResourceDeposit>(app.world_mut()), deposits);
    assert_eq!(count::<StrategicController>(app.world_mut()), controllers);
}

fn request_start(app: &mut App, scenario: Scenario) {
    app.world_mut().write_message(StartScenario(scenario));
    app.update();
}

fn has_owned_intent(app: &App, owner: SwarmId) -> bool {
    app.world()
        .resource::<IntentGrid>()
        .iter_active_cells()
        .any(|(_, intent)| {
            IntentKind::ALL
                .into_iter()
                .any(|kind| intent.has_owned(kind, owner))
        })
}

fn assert_standard_opponent_recovers_owned_intent_within_one_second(app: &mut App) {
    let opponent = app
        .world_mut()
        .query_filtered::<&SwarmId, (With<Swarm>, With<StrategicController>)>()
        .single(app.world())
        .copied()
        .expect("Standard starts one controlled opponent swarm");
    let owned = app.world().resource::<IntentGrid>().swarm_tiles(opponent);
    for cell in owned {
        for kind in IntentKind::ALL {
            app.world_mut()
                .resource_mut::<IntentGrid>()
                .erase(cell, kind, opponent);
        }
    }

    let deadline = app.world().resource::<Time<Fixed>>().elapsed() + Duration::from_secs(1);
    while app.world().resource::<Time<Fixed>>().elapsed() < deadline {
        app.update();
        if has_owned_intent(app, opponent) {
            return;
        }
    }

    panic!("the normal Standard opponent did not recover owned intent within one simulated second");
}

fn has_spectator_label(world: &mut World) -> bool {
    world
        .query::<&Text>()
        .iter(world)
        .any(|text| text.0 == "Spectating | Painting disabled")
}

fn click(app: &mut App, action: MenuAction) {
    let button = app
        .world_mut()
        .query::<(Entity, &MenuAction)>()
        .iter(app.world())
        .find_map(|(entity, candidate)| (*candidate == action).then_some(entity))
        .expect("menu action is wired to a button");
    app.world_mut()
        .entity_mut(button)
        .insert(Interaction::Pressed);
    app.update();
    app.world_mut().entity_mut(button).insert(Interaction::None);
}

#[test]
fn menu_start_replaces_the_match_and_keeps_the_application_shell() {
    let output = TemporaryDirectory::new();
    let (mut app, camera) = session_app(output.path());
    assert_scenario_shape(&mut app, Scenario::Standard);
    assert_eq!(app.world().resource::<SessionGeneration>().0, 0);

    app.world_mut().resource_mut::<ScenarioMenu>().toggle();
    app.update();
    click(&mut app, MenuAction::Select(Scenario::Sandbox));
    click(&mut app, MenuAction::Start);

    assert_scenario_shape(&mut app, Scenario::Sandbox);
    assert_eq!(app.world().resource::<SessionGeneration>().0, 1);
    assert!(!app.world().resource::<ScenarioMenu>().open);
    assert!(
        app.world().get_entity(camera).is_ok(),
        "the application camera must survive a session replacement"
    );
    let ticks = app.world().resource::<SimulatedTicks>().0;
    app.update();
    assert!(
        app.world().resource::<SimulatedTicks>().0 > ticks,
        "the freshly started session must resume simulation"
    );
}

#[test]
fn intent_panel_tracks_the_scenario_across_same_process_restarts() {
    let output = TemporaryDirectory::new();
    let (mut app, _) = session_app(output.path());
    assert_eq!(
        count::<IntentLayerButton>(app.world_mut()),
        IntentKind::COUNT
    );
    assert!(!has_spectator_label(app.world_mut()));

    request_start(&mut app, Scenario::AiBattle);

    assert_eq!(count::<IntentLayerButton>(app.world_mut()), 0);
    assert!(has_spectator_label(app.world_mut()));

    request_start(&mut app, Scenario::Standard);

    assert_eq!(
        count::<IntentLayerButton>(app.world_mut()),
        IntentKind::COUNT
    );
    assert!(!has_spectator_label(app.world_mut()));
}

#[test]
fn standard_opponent_recovers_owned_intent_after_startup_and_restart() {
    let output = TemporaryDirectory::new();
    let (mut app, _) = session_app(output.path());

    assert_standard_opponent_recovers_owned_intent_within_one_second(&mut app);
    request_start(&mut app, Scenario::Sandbox);
    assert_scenario_shape(&mut app, Scenario::Sandbox);
    request_start(&mut app, Scenario::Standard);
    assert_standard_opponent_recovers_owned_intent_within_one_second(&mut app);
}

#[test]
fn normal_scenarios_restore_deliberate_pacing_after_a_baseline_ai_battle() {
    let output = TemporaryDirectory::new();
    let (mut app, _) = session_app(output.path());
    let deliberate = GameplayPacing::from(PacingId::Deliberate);
    let baseline = GameplayPacing::from(PacingId::Baseline);

    assert_eq!(*app.world().resource::<GameplayPacing>(), deliberate);

    app.world_mut()
        .resource_mut::<BattleExperimentConfig>()
        .pacing = PacingId::Baseline;
    request_start(&mut app, Scenario::AiBattle);
    assert_eq!(*app.world().resource::<GameplayPacing>(), baseline);

    request_start(&mut app, Scenario::Sandbox);
    assert_eq!(*app.world().resource::<GameplayPacing>(), deliberate);

    request_start(&mut app, Scenario::AiBattle);
    assert_eq!(*app.world().resource::<GameplayPacing>(), baseline);

    request_start(&mut app, Scenario::Standard);
    assert_eq!(*app.world().resource::<GameplayPacing>(), deliberate);
}

#[test]
fn restarting_clears_mutated_session_state_and_rebuilds_the_selected_scenario() {
    let output = TemporaryDirectory::new();
    let (mut app, camera) = session_app(output.path());
    let stale_runtime_bot = app.world_mut().spawn(Nanobot {}).id();
    assert!(
        app.world()
            .entity(stale_runtime_bot)
            .contains::<SessionEntity>(),
        "runtime-spawned gameplay entities must automatically join the session lifetime"
    );
    app.world_mut()
        .resource_mut::<ResourceLedger>()
        .add(ResourceKind::Minerals, 73);
    app.world_mut().resource_mut::<IntentGrid>().paint(
        IVec2::new(100, 100),
        IntentKind::Corridor,
        SwarmId::PLAYER,
    );
    *app.world_mut().resource_mut::<MatchOutcome>() = MatchOutcome::Draw;
    {
        let mut entity = app.world_mut().entity_mut(camera);
        entity.get_mut::<Transform>().unwrap().translation += Vec3::new(900.0, -400.0, 0.0);
        entity.get_mut::<CameraZoom2d>().unwrap().zoom = 17.0;
        entity.get_mut::<FlyCamera2d>().unwrap().velocity = Vec2::new(8.0, 3.0);
        let Projection::Orthographic(mut projection) =
            entity.get_mut::<Projection>().unwrap().clone()
        else {
            panic!("fixture camera is orthographic")
        };
        projection.scale = 17.0;
        entity.insert(Projection::Orthographic(projection));
    }

    request_start(&mut app, Scenario::Standard);

    assert_scenario_shape(&mut app, Scenario::Standard);
    assert_eq!(app.world().resource::<SessionGeneration>().0, 1);
    assert!(app.world().get_entity(stale_runtime_bot).is_err());
    assert_eq!(
        app.world()
            .resource::<ResourceLedger>()
            .total(ResourceKind::Minerals),
        0
    );
    assert!(
        !app.world()
            .resource::<IntentGrid>()
            .cell(IVec2::new(100, 100))
            .unwrap()
            .has(IntentKind::Corridor)
    );
    assert_eq!(
        *app.world().resource::<MatchOutcome>(),
        MatchOutcome::InProgress
    );
    let camera_entity = app.world().entity(camera);
    assert_eq!(
        camera_entity.get::<Transform>().unwrap().translation,
        cell_origin(PLAYER_CELL).extend(0.0)
    );
    assert_eq!(
        camera_entity.get::<CameraZoom2d>().unwrap().zoom,
        DEFAULT_CAMERA_ZOOM
    );
    assert_eq!(
        camera_entity.get::<FlyCamera2d>().unwrap().velocity,
        Vec2::ZERO
    );
    let Projection::Orthographic(projection) = camera_entity.get::<Projection>().unwrap() else {
        panic!("fixture camera is orthographic")
    };
    assert_eq!(projection.scale, DEFAULT_CAMERA_ZOOM);

    request_start(&mut app, Scenario::AiBattle);
    assert_scenario_shape(&mut app, Scenario::AiBattle);
    assert_eq!(app.world().resource::<SessionGeneration>().0, 2);
}

fn persisted_summary(directory: &Path) -> BattleSummary {
    let bytes = std::fs::read(directory.join("summary.json")).expect("battle summary is persisted");
    serde_json::from_slice(&bytes).expect("battle summary uses its public JSON schema")
}

#[test]
fn replacing_ai_battles_interrupts_active_runs_and_preserves_completed_runs() {
    let output = TemporaryDirectory::new();
    let (mut app, _) = session_app(output.path());
    request_start(&mut app, Scenario::AiBattle);
    let interrupted_directory = app.world().resource::<BattleRun>().directory.clone();

    request_start(&mut app, Scenario::AiBattle);
    let replacement_directory = app.world().resource::<BattleRun>().directory.clone();
    assert_ne!(
        replacement_directory, interrupted_directory,
        "each AI battle must receive an independent recording"
    );
    assert_eq!(
        persisted_summary(&interrupted_directory).status,
        RunStatus::Interrupted
    );

    *app.world_mut().resource_mut::<MatchOutcome>() = MatchOutcome::Winner(SwarmId::PLAYER);
    app.update();
    assert_eq!(
        app.world().resource::<BattleRun>().summary.status,
        RunStatus::Completed
    );
    request_start(&mut app, Scenario::Sandbox);
    assert!(!app.world().contains_resource::<BattleRun>());
    assert_eq!(
        persisted_summary(&replacement_directory).status,
        RunStatus::Completed
    );
    assert_scenario_shape(&mut app, Scenario::Sandbox);
}
