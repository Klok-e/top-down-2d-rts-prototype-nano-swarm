use std::time::Duration;

use bevy::{
    app::{AppLabel, SubApp},
    prelude::*,
    time::{TimePlugin, TimeUpdateStrategy},
};
use serde_json::Value;
use top_down_2d_rts_prototype_nano_swarm::{
    Presentation,
    agent_control::{
        AgentControlCorePlugin, AgentControlHandle, AgentResponse, parse_request_line,
    },
    build_app_with_presentation, fixed_simulation_time,
    nanobot::{Nanobot, Structure},
    perf::{HitchRecorderPlugin, RecorderClock},
    runtime::HeadlessPacing,
    scenario_selection::{Scenario, ScenarioSelection},
    session_lifecycle::StartScenario,
};

#[derive(Resource, Default)]
struct FrameWork {
    update: Duration,
    post_update: Duration,
    fixed_update: Duration,
    render: Duration,
}

#[derive(AppLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TestRender;

struct RecorderApp {
    app: App,
    control: AgentControlHandle,
}

impl RecorderApp {
    fn new() -> Self {
        let mut app = App::new();
        app.add_plugins(TimePlugin);
        Self::with_app(app)
    }

    fn with_app(mut app: App) -> Self {
        let (control, channel) = AgentControlCorePlugin::channel(4);
        app.insert_resource(fixed_simulation_time())
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(RecorderClock::manual())
            .init_resource::<FrameWork>()
            .add_plugins((HitchRecorderPlugin, channel))
            .add_systems(
                FixedUpdate,
                |work: Res<FrameWork>, mut clock: ResMut<RecorderClock>| {
                    clock.advance(work.fixed_update);
                },
            )
            .add_systems(
                Update,
                |work: Res<FrameWork>, mut clock: ResMut<RecorderClock>| {
                    clock.advance(work.update);
                },
            )
            .add_systems(
                PostUpdate,
                |work: Res<FrameWork>, mut clock: ResMut<RecorderClock>| {
                    clock.advance(work.post_update);
                },
            );
        let mut render = SubApp::new();
        render.set_extract(|main, _| {
            let duration = main.resource::<FrameWork>().render;
            main.resource_mut::<RecorderClock>().advance(duration);
        });
        app.insert_sub_app(TestRender, render);
        Self { app, control }
    }

    fn advance_frames(&mut self, count: usize) {
        for _ in 0..count {
            self.app.update();
        }
    }

    fn frame_with_work(&mut self, update_ms: u64, post_update_ms: u64, render: Duration) {
        self.run_frame(FrameWork {
            update: Duration::from_millis(update_ms),
            post_update: Duration::from_millis(post_update_ms),
            render,
            ..default()
        });
    }

    fn run_frame(&mut self, work: FrameWork) {
        self.app.insert_resource(work);
        self.app.update();
        self.app.insert_resource(FrameWork::default());
    }

    fn request(&mut self, line: &str) -> AgentResponse {
        let request = parse_request_line(line.as_bytes()).unwrap();
        let reply = self.control.submit(request).unwrap();
        self.app.update();
        let response = reply
            .try_recv()
            .expect("immediate command replies in one update");
        assert!(response.ok, "{response:?}");
        response
    }

    fn hitches(&mut self) -> AgentResponse {
        self.request(r#"{"id":1,"method":"perf.hitches"}"#)
    }
}

fn episodes(result: &Value) -> &[Value] {
    result["episodes"].as_array().unwrap()
}

#[test]
fn a_slow_frame_reports_full_context_and_attributes_only_main_schedule_time() {
    let mut game = RecorderApp::new();
    game.app.world_mut().spawn(Nanobot {});
    game.app.world_mut().spawn(Nanobot {});
    game.app.world_mut().spawn(Structure::default());
    game.advance_frames(123);
    game.frame_with_work(0, 0, Duration::from_micros(33_300));
    game.frame_with_work(7, 5, Duration::from_millis(23));
    game.advance_frames(29);
    assert!(episodes(&game.hitches().result.unwrap()).is_empty());

    let response = game.hitches();
    let result = response.result.unwrap();
    assert_eq!(result["dropped"], 0);
    let [episode] = episodes(&result) else {
        panic!("expected one closed hitch episode");
    };
    assert_eq!(episode["first_hitch_frame"], 125);
    assert_eq!(episode["last_hitch_frame"], 125);
    let frames = episode["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 151);
    assert_eq!(frames[0]["frame"], 5);
    assert_eq!(frames[150]["frame"], response.frame);
    assert_eq!(frames[150]["frame"], 155);
    assert_eq!(frames[150]["fixed_tick"], response.fixed_tick);

    let slow = &frames[120];
    assert_eq!(slow["frame"], 125);
    assert_eq!(slow["fixed_ticks_run"], 1);
    assert_eq!(slow["frame_ns"], 35_000_000);
    assert_eq!(slow["schedule_ns"]["Update"], 7_000_000);
    assert_eq!(slow["schedule_ns"]["PostUpdate"], 5_000_000);
    assert_eq!(slow["unmeasured_ns"], 23_000_000);
    assert_eq!(slow["nanobots"], 2);
    assert_eq!(slow["structures"], 1);
}

#[test]
fn actual_bevy_catch_up_is_a_hitch_below_the_frame_time_threshold() {
    let mut game = RecorderApp::new();
    game.advance_frames(61);
    let timestep = fixed_simulation_time().timestep();
    game.app
        .insert_resource(TimeUpdateStrategy::ManualDuration(timestep * 2));
    game.frame_with_work(0, 0, Duration::from_millis(20));
    game.app
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    game.advance_frames(30);

    let result = game.hitches().result.unwrap();
    let [episode] = episodes(&result) else {
        panic!("expected one catch-up episode");
    };
    assert_eq!(episode["first_hitch_frame"], 62);
    let frames = episode["frames"].as_array().unwrap();
    assert_eq!(frames[61]["frame_ns"], 20_000_000);
    assert_eq!(frames[61]["fixed_ticks_run"], 2);
    assert_eq!(
        frames[61]["fixed_tick"].as_u64().unwrap(),
        frames[60]["fixed_tick"].as_u64().unwrap() + 2,
    );
}

#[test]
fn nearby_hitches_share_an_episode_and_the_cursor_skips_episodes_already_read() {
    let mut game = RecorderApp::new();
    let hitch = Duration::from_millis(34);
    game.advance_frames(60);
    game.frame_with_work(0, 0, hitch);
    game.advance_frames(27);
    game.frame_with_work(0, 0, hitch);
    game.advance_frames(30);

    let result = game.hitches().result.unwrap();
    let [first] = episodes(&result) else {
        panic!("expected the nearby hitches to share an episode");
    };
    assert_eq!(first["first_hitch_frame"], 61);
    assert_eq!(first["last_hitch_frame"], 89);
    let first_id = first["id"].as_u64().unwrap();

    game.frame_with_work(0, 0, hitch);
    game.advance_frames(30);
    let result = game
        .request(&format!(
            r#"{{"id":1,"method":"perf.hitches","params":{{"after_id":{first_id}}}}}"#
        ))
        .result
        .unwrap();
    let [second] = episodes(&result) else {
        panic!("expected only the episode after the cursor");
    };
    assert_eq!(second["first_hitch_frame"], 121);
    let second_id = second["id"].as_u64().unwrap();
    assert!(second_id > first_id);
    let result = game
        .request(&format!(
            r#"{{"id":1,"method":"perf.hitches","params":{{"after_id":{second_id}}}}}"#
        ))
        .result
        .unwrap();
    assert!(episodes(&result).is_empty());
}

#[test]
fn startup_warmup_excludes_frame_60_and_does_not_exclude_frame_61() {
    let mut game = RecorderApp::new();
    let hitch = Duration::from_millis(34);
    game.advance_frames(59);
    game.frame_with_work(0, 0, hitch);
    game.frame_with_work(0, 0, hitch);
    game.advance_frames(30);

    let result = game.hitches().result.unwrap();
    let [episode] = episodes(&result) else {
        panic!("expected one episode after startup warm-up");
    };
    assert_eq!(episode["first_hitch_frame"], 61);
    assert_eq!(episode["last_hitch_frame"], 61);
}

#[test]
fn paused_frames_do_not_start_an_episode() {
    let mut game = RecorderApp::new();
    let hitch = Duration::from_millis(34);
    game.advance_frames(60);
    game.app.world_mut().resource_mut::<Time<Virtual>>().pause();
    game.frame_with_work(0, 0, hitch);
    game.advance_frames(31);
    assert!(episodes(&game.hitches().result.unwrap()).is_empty());

    game.app
        .world_mut()
        .resource_mut::<Time<Virtual>>()
        .unpause();
    game.frame_with_work(0, 0, hitch);
    game.advance_frames(30);
    let result = game.hitches().result.unwrap();
    let [episode] = episodes(&result) else {
        panic!("expected one hitch after unpausing");
    };
    assert_eq!(episode["first_hitch_frame"], 94);
    assert_eq!(episode["last_hitch_frame"], 94);
}

#[test]
fn restarting_standard_and_switching_scenarios_each_restart_the_warmup() {
    let mut game = RecorderApp::with_app(build_app_with_presentation(Presentation::SimulationOnly));
    let hitch = Duration::from_millis(34);
    game.advance_frames(90);

    let mut after_id = 0;
    for scenario in [Scenario::Standard, Scenario::Sandbox] {
        let frame_before_start = game.hitches().frame;
        game.app.world_mut().write_message(StartScenario(scenario));
        game.frame_with_work(0, 0, hitch);
        assert_eq!(
            game.app.world().resource::<ScenarioSelection>().current,
            scenario
        );
        game.advance_frames(58);
        game.frame_with_work(0, 0, hitch);
        game.frame_with_work(0, 0, hitch);
        game.advance_frames(30);

        let result = game
            .request(&format!(
                r#"{{"id":1,"method":"perf.hitches","params":{{"after_id":{after_id}}}}}"#
            ))
            .result
            .unwrap();
        let [episode] = episodes(&result) else {
            panic!("expected only the episode after the {scenario:?} warm-up");
        };
        let first_frame = frame_before_start + 2;
        assert_eq!(episode["frames"][0]["frame"], first_frame);
        assert_eq!(episode["first_hitch_frame"], first_frame + 60);
        assert_eq!(episode["last_hitch_frame"], first_frame + 60);
        after_id = episode["id"].as_u64().unwrap();
    }
}

#[test]
fn unbounded_mode_classifies_fixed_tick_cost_instead_of_render_time() {
    let mut game = RecorderApp::new();
    game.app.insert_resource(HeadlessPacing { unbounded: true });
    game.advance_frames(60);
    game.frame_with_work(0, 0, Duration::from_millis(40));
    game.run_frame(FrameWork {
        fixed_update: Duration::from_nanos(33_300_000),
        ..default()
    });
    game.run_frame(FrameWork {
        fixed_update: Duration::from_nanos(33_300_001),
        ..default()
    });
    game.advance_frames(30);

    let result = game.hitches().result.unwrap();
    let [episode] = episodes(&result) else {
        panic!("expected one fixed-tick hitch");
    };
    assert_eq!(episode["first_hitch_frame"], 63);
    assert_eq!(episode["last_hitch_frame"], 63);
}
