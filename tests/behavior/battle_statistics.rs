use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use top_down_2d_rts_prototype_nano_swarm::{
    battle_experiment::{BattleExperimentConfig, ControllerId, LayoutId, PacingId},
    battle_statistics::{BattleStatisticsConfig, BattleStatisticsPlugin},
    gameplay_pacing::GameplayPacing,
    nanobot::NanobotType,
    scenario_selection::{Scenario, ScenarioSelection},
};
#[path = "../common/mod.rs"]
mod common;

struct Output(PathBuf);
impl Output {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "nano-battle-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn summary(&self) -> serde_json::Value {
        let directory = std::fs::read_dir(&self.0)
            .expect("battle must create a run directory")
            .next()
            .unwrap()
            .unwrap()
            .path();
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap()
    }

    fn samples_csv(&self) -> String {
        let directory = std::fs::read_dir(&self.0)
            .expect("battle must create a run directory")
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::read_to_string(directory.join("samples.csv")).unwrap()
    }
}

#[test]
fn experiment_cutoff_saves_an_unresolved_frozen_result_without_a_match_outcome() {
    let output = Output::new();
    let mut app = battle(&output);
    app.insert_resource(BattleExperimentConfig {
        controllers: [ControllerId::Adaptive, ControllerId::Timed],
        layout: LayoutId::Narrows,
        swap_sides: true,
        pacing: PacingId::Deliberate,
        cutoff_seconds: Some(1),
        realtime: true,
    });
    app.insert_resource(GameplayPacing::from(PacingId::Deliberate));
    app.world_mut()
        .resource_mut::<BattleStatisticsConfig>()
        .headless = true;

    for _ in 0..60 {
        app.update();
    }

    let terminal = output.summary();
    assert_eq!(terminal["status"], "unresolved");
    assert!(terminal["outcome"].is_null());
    assert_eq!(terminal["latest"]["fixed_tick"], 60);
    assert_eq!(terminal["experiment"]["controllers"][0], "adaptive");
    assert_eq!(terminal["experiment"]["controllers"][1], "timed");
    assert_eq!(terminal["experiment"]["layout"], "narrows");
    assert_eq!(terminal["experiment"]["swap_sides"], true);
    assert_eq!(terminal["experiment"]["pacing"], "deliberate");
    assert_eq!(terminal["experiment"]["cutoff_seconds"], 1);
    assert_eq!(terminal["experiment"]["realtime"], true);
    assert_eq!(terminal["gameplay_pacing"]["construction_work_ticks"], 90);
    assert_eq!(terminal["gameplay_pacing"]["attack_interval_ticks"], 30);
    assert_eq!(
        terminal["gameplay_pacing"]["charge_drain_per_tick"],
        0.000125
    );
    assert_eq!(app.should_exit(), Some(bevy::app::AppExit::Success));

    for _ in 0..60 {
        app.update();
    }
    assert_eq!(output.summary(), terminal);
}

#[test]
fn normal_ai_battle_remains_in_progress_beyond_the_experiment_budget() {
    let output = Output::new();
    let mut app = battle(&output);

    for _ in 0..36_060 {
        app.update();
    }

    let summary = output.summary();
    assert_eq!(summary["status"], "in_progress");
    assert!(summary["outcome"].is_null());
    assert_eq!(
        summary["experiment"]["cutoff_seconds"],
        serde_json::Value::Null
    );
    assert!(summary["latest"]["simulation_seconds"].as_f64().unwrap() >= 601.0);
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn battle(output: &Output) -> App {
    let mut app = common::sim_app_with_elimination();
    let mut selection = ScenarioSelection::default();
    selection.current = Scenario::AiBattle;
    app.insert_resource(selection);
    app.insert_resource(top_down_2d_rts_prototype_nano_swarm::fixed_simulation_time());
    app.insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    app.insert_resource(BattleStatisticsConfig {
        output_root: output.0.clone(),
        ..default()
    });
    app.add_plugins(BattleStatisticsPlugin);
    common::spawn_swarm_with_nanobots(&mut app, Vec2::ZERO, &[(NanobotType::Worker, 1)]);
    common::spawn_opponent_swarm_with_nanobots(
        &mut app,
        Vec2::X * 500.,
        &[(NanobotType::Defender, 1)],
    );
    app
}

#[test]
fn battle_persists_per_second_samples_using_simulation_time() {
    let output = Output::new();
    let mut app = battle(&output);
    for _ in 0..60 {
        app.update();
    }
    let summary = output.summary();
    assert_eq!(summary["status"], "in_progress");
    assert_eq!(summary["latest"]["swarms"]["0"]["workers"], 1);
    assert_eq!(summary["latest"]["swarms"]["1"]["defenders"], 1);
    assert!((summary["latest"]["simulation_seconds"].as_f64().unwrap() - 1.).abs() < 0.001);
    assert_eq!(summary["latest"]["tick_timing"]["count"], 60);
    assert!(
        summary["latest"]["frame_timing"]["count"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "watchable runs need frame timing"
    );
}

#[test]
fn headless_frame_timing_is_recorded_only_for_realtime_experiments() {
    for (realtime, expects_frame_timing) in [(false, false), (true, true)] {
        let output = Output::new();
        let mut app = battle(&output);
        app.world_mut()
            .resource_mut::<BattleStatisticsConfig>()
            .headless = true;
        app.insert_resource(BattleExperimentConfig {
            realtime,
            ..default()
        });

        for _ in 0..60 {
            app.update();
        }

        let summary = output.summary();
        let csv = output.samples_csv();
        let frame_peak_column = csv
            .lines()
            .next()
            .unwrap()
            .split(',')
            .position(|name| name == "frame_max_ms")
            .expect("frame peaks must remain available after later samples replace the summary");
        let frame_peak = csv
            .lines()
            .last()
            .unwrap()
            .split(',')
            .nth(frame_peak_column)
            .unwrap();
        if expects_frame_timing {
            assert!(
                summary["latest"]["frame_timing"]["count"]
                    .as_u64()
                    .is_some_and(|count| count > 0)
            );
            let recorded_peak = frame_peak.parse::<f64>().unwrap();
            let latest_peak = summary["latest"]["frame_timing"]["max_ms"]
                .as_f64()
                .unwrap();
            assert!((recorded_peak - latest_peak).abs() <= 1e-9 * latest_peak.abs().max(1.0));
        } else {
            assert!(summary["latest"]["frame_timing"].is_null());
            assert!(frame_peak.is_empty());
        }
    }
}

#[test]
fn csv_names_the_recent_p95_window_sample_count() {
    let output = Output::new();
    let mut app = battle(&output);
    for _ in 0..60 {
        app.update();
    }

    let header = output.samples_csv().lines().next().unwrap().to_string();
    assert!(header.contains(",controller_p95_ms,controller_p95_window_samples,controller_max_ms,"));
}

#[test]
fn battle_end_flushes_partial_interval_and_freezes_results_while_world_advances() {
    use top_down_2d_rts_prototype_nano_swarm::nanobot::{Health, SwarmMember};
    let output = Output::new();
    let mut app = battle(&output);
    for _ in 0..10 {
        app.update();
    }
    for (owner, mut health) in app
        .world_mut()
        .query::<(&SwarmMember, &mut Health)>()
        .iter_mut(app.world_mut())
    {
        if !owner.0.is_player() {
            health.current = 0;
        }
    }
    app.update();
    let terminal = output.summary();
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["outcome"], "swarm_0_wins");
    assert_eq!(terminal["latest"]["fixed_tick"], 11);
    assert_eq!(terminal["latest"]["swarms"]["1"]["population"], 0);
    assert!(
        terminal["latest"]["swarms"]["1"]["elimination_seconds"]
            .as_f64()
            .unwrap()
            > 0.
    );
    let elapsed = app.world().resource::<Time<Fixed>>().elapsed();
    for _ in 0..120 {
        app.update();
    }
    assert!(app.world().resource::<Time<Fixed>>().elapsed() > elapsed);
    assert_eq!(
        output.summary(),
        terminal,
        "post-battle updates must not rewrite results"
    );
}

#[test]
fn interruption_saves_partial_statistics_without_inventing_an_outcome() {
    let output = Output::new();
    let mut app = battle(&output);
    for _ in 0..7 {
        app.update();
    }
    app.world_mut().write_message(AppExit::from_code(130));
    app.update();
    let summary = output.summary();
    assert_eq!(summary["status"], "interrupted");
    assert!(summary["outcome"].is_null());
    assert_eq!(summary["latest"]["fixed_tick"], 8);
    assert_eq!(summary["latest"]["swarms"]["0"]["population"], 1);
}

#[test]
fn samples_preserve_gross_events_and_stop_counters_after_draw() {
    use top_down_2d_rts_prototype_nano_swarm::{
        battle_statistics::{BattleCounters, BattleEvent},
        nanobot::{Health, SwarmId},
    };
    let output = Output::new();
    let mut app = battle(&output);
    app.update();
    {
        let mut counters = app.world_mut().resource_mut::<BattleCounters>();
        for event in [
            BattleEvent::Birth,
            BattleEvent::Death,
            BattleEvent::Gathered(30),
            BattleEvent::Consumed(30),
            BattleEvent::StructureBuilt,
            BattleEvent::StructureLost,
        ] {
            counters.record(SwarmId::PLAYER, event);
        }
    }
    for _ in 1..60 {
        app.update();
    }
    let summary = output.summary();
    for field in ["births", "deaths", "structures_built", "structures_lost"] {
        assert_eq!(
            summary["latest"]["swarms"]["0"][field], 1,
            "gross {field} must survive a net-zero interval"
        );
    }
    assert_eq!(summary["latest"]["swarms"]["0"]["minerals_gathered"], 30);
    assert_eq!(summary["latest"]["swarms"]["0"]["minerals_consumed"], 30);
    assert_eq!(summary["schema_version"], 3);
    assert_eq!(
        summary["latest"]["swarms"]["0"]["effective_damage_total"],
        0
    );
    assert_eq!(
        summary["latest"]["swarms"]["0"]["effective_damage_nanobots"],
        0
    );
    assert_eq!(
        summary["latest"]["swarms"]["0"]["effective_damage_structures"], 0,
        "non-combat structure loss must not count as hostile combat damage",
    );
    assert_eq!(summary["latest"]["swarms"]["0"]["scored_damage_total"], 0);
    assert_eq!(
        summary["latest"]["swarms"]["0"]["scored_damage_nanobots"],
        0
    );
    assert_eq!(
        summary["latest"]["swarms"]["0"]["scored_damage_structures"],
        0
    );
    assert_eq!(summary["latest"]["swarms"]["1"]["births"], 0);
    for mut health in app
        .world_mut()
        .query::<&mut Health>()
        .iter_mut(app.world_mut())
    {
        health.current = 0;
    }
    app.update();
    let completed = output.summary();
    assert_eq!(completed["outcome"], "draw");
    assert_eq!(
        completed["latest"]["swarms"]["0"]["elimination_seconds"],
        completed["latest"]["swarms"]["1"]["elimination_seconds"]
    );
    app.world_mut()
        .resource_mut::<BattleCounters>()
        .record(SwarmId::PLAYER, BattleEvent::Birth);
    assert_eq!(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER)
            .births,
        1
    );
    let frozen = serde_json::to_value(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER),
    )
    .unwrap();
    assert_eq!(frozen["effective_damage_total"], 0);
    assert_eq!(frozen["scored_damage_total"], 0);
    app.world_mut().write_message(AppExit::from_code(130));
    app.update();
    assert_eq!(
        output.summary(),
        completed,
        "interrupting a completed battle preserves its result"
    );
}

#[test]
fn effective_damage_is_cumulative_in_json_and_csv_and_freezes_at_cutoff() {
    use top_down_2d_rts_prototype_nano_swarm::{
        battle_statistics::{BattleCounters, BattleEvent},
        nanobot::SwarmId,
    };
    let output = Output::new();
    let mut app = battle(&output);
    app.insert_resource(BattleExperimentConfig {
        cutoff_seconds: Some(1),
        ..default()
    });
    app.update();
    {
        let mut counters = app.world_mut().resource_mut::<BattleCounters>();
        counters.record(SwarmId::PLAYER, BattleEvent::EffectiveNanobotDamage(17));
        counters.record(SwarmId::PLAYER, BattleEvent::EffectiveStructureDamage(5));
        counters.record(SwarmId::PLAYER, BattleEvent::ScoredNanobotDamage(13));
        counters.record(SwarmId::PLAYER, BattleEvent::ScoredStructureDamage(3));
    }
    for _ in 1..60 {
        app.update();
    }

    let terminal = output.summary();
    assert_eq!(terminal["schema_version"], 3);
    let swarm = &terminal["latest"]["swarms"]["0"];
    assert_eq!(swarm["effective_damage_total"], 22);
    assert_eq!(swarm["effective_damage_nanobots"], 17);
    assert_eq!(swarm["effective_damage_structures"], 5);
    assert_eq!(swarm["scored_damage_total"], 16);
    assert_eq!(swarm["scored_damage_nanobots"], 13);
    assert_eq!(swarm["scored_damage_structures"], 3);

    let csv = output.samples_csv();
    let header = csv.lines().next().unwrap().split(',').collect::<Vec<_>>();
    let row = csv
        .lines()
        .skip(1)
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|columns| columns.get(2) == Some(&"0"))
        .expect("the cutoff sample must contain the player swarm");
    for (field, expected) in [
        ("effective_damage_total", "22"),
        ("effective_damage_nanobots", "17"),
        ("effective_damage_structures", "5"),
        ("scored_damage_total", "16"),
        ("scored_damage_nanobots", "13"),
        ("scored_damage_structures", "3"),
    ] {
        let index = header.iter().position(|name| *name == field).unwrap();
        assert_eq!(row[index], expected);
    }

    app.world_mut()
        .resource_mut::<BattleCounters>()
        .record(SwarmId::PLAYER, BattleEvent::EffectiveNanobotDamage(100));
    app.world_mut()
        .resource_mut::<BattleCounters>()
        .record(SwarmId::PLAYER, BattleEvent::ScoredNanobotDamage(100));
    let frozen = serde_json::to_value(
        app.world()
            .resource::<BattleCounters>()
            .totals_for(SwarmId::PLAYER),
    )
    .unwrap();
    assert_eq!(frozen["effective_damage_total"], 22);
    assert_eq!(frozen["scored_damage_total"], 16);
}

#[test]
fn frame_statistics_include_frames_without_fixed_ticks() {
    let output = Output::new();
    let mut app = battle(&output);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_nanos(8_333_334),
    ));
    for _ in 0..120 {
        app.update();
    }
    let summary = output.summary();
    assert_eq!(summary["latest"]["tick_timing"]["count"], 60);
    assert!(
        summary["latest"]["frame_timing"]["count"].as_u64().unwrap() > 100,
        "120 application frames must not be reduced to the 60 frames containing fixed ticks"
    );
}

#[test]
fn recording_uses_session_policy_without_a_scenario_selection() {
    use top_down_2d_rts_prototype_nano_swarm::session::SessionRules;
    for enabled in [false, true] {
        let output = Output::new();
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(top_down_2d_rts_prototype_nano_swarm::fixed_simulation_time())
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(SessionRules {
                scenario_name: "observation",
                record_statistics: enabled,
                ..default()
            })
            .insert_resource(BattleStatisticsConfig {
                output_root: output.0.clone(),
                ..default()
            })
            .add_plugins(BattleStatisticsPlugin);
        for _ in 0..61 {
            app.update();
        }
        if enabled {
            assert_eq!(output.summary()["scenario"], "observation");
            assert_eq!(output.summary()["samples"], 1);
        } else {
            assert!(
                !output.0.exists(),
                "disabled recording must not create output"
            );
        }
    }
}

#[test]
fn failed_summary_write_signals_failure_and_preserves_last_flushed_summary() {
    let output = Output::new();
    let mut app = battle(&output);
    app.update();
    let previous = output.summary();
    let directory = std::fs::read_dir(&output.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::create_dir(directory.join("summary.json.tmp")).unwrap();
    for _ in 1..60 {
        app.update();
    }
    assert_eq!(app.should_exit(), Some(AppExit::error()));
    assert_eq!(
        output.summary(),
        previous,
        "a failed atomic replacement must preserve the preceding summary"
    );
}

#[test]
fn recorded_outcomes_use_actual_swarm_ids_and_session_perspective() {
    use top_down_2d_rts_prototype_nano_swarm::{
        nanobot::{Health, SwarmId, SwarmMember},
        session::{OutcomeMode, SessionRules},
    };
    for (mode, expected) in [
        (OutcomeMode::SwarmRelative, "swarm_42_wins"),
        (OutcomeMode::PlayerRelative, "victory"),
    ] {
        let output = Output::new();
        let mut app = battle(&output);
        app.update();
        {
            let mut rules = app.world_mut().resource_mut::<SessionRules>();
            rules.player_swarm = Some(SwarmId(42));
            rules.outcomes = mode;
        }
        for mut id in app
            .world_mut()
            .query::<&mut SwarmId>()
            .iter_mut(app.world_mut())
        {
            id.0 = if id.0 == 0 { 7 } else { 42 };
        }
        for (mut member, mut health) in app
            .world_mut()
            .query::<(&mut SwarmMember, &mut Health)>()
            .iter_mut(app.world_mut())
        {
            if member.0.0 == 0 {
                member.0 = SwarmId(7);
                health.current = 0;
            } else {
                member.0 = SwarmId(42);
            }
        }
        app.update();
        let saved = output.summary();
        assert_eq!(saved["outcome"], expected);
        assert!(
            saved["latest"]["swarms"]["7"]["elimination_seconds"]
                .as_f64()
                .unwrap()
                > 0.
        );
        assert!(saved["latest"]["swarms"]["42"]["elimination_seconds"].is_null());
    }
}
