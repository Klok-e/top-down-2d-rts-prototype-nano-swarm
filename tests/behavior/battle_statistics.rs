use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use top_down_2d_rts_prototype_nano_swarm::{
    battle_statistics::{BattleStatisticsConfig, BattleStatisticsPlugin},
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
    app.world_mut().write_message(AppExit::from_code(130));
    app.update();
    assert_eq!(
        output.summary(),
        completed,
        "interrupting a completed battle preserves its result"
    );
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
