use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::{app::AppExit, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{Nanobot, OwnerSwarm, Swarm, SwarmId, SwarmMember},
    runtime::{RuntimeOptions, build_runtime_app},
};

struct RunDirectory(PathBuf);

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires a GPU adapter; run with cargo test --test playtest accelerated_battle_runtime -- --ignored"]
fn accelerated_battle_runtime_saves_terminal_results_before_requesting_exit() {
    let directory = RunDirectory(std::env::temp_dir().join(format!(
            "nano-battle-runtime-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let mut app = build_runtime_app(
        RuntimeOptions::parse([
            "--headless",
            "--scenario",
            "ai-battle",
            "--seed",
            "42",
            "--width",
            "320",
            "--height",
            "180",
            "--output-root",
            directory.0.to_str().unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    while app.plugins_state() == bevy::app::PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    app.update();
    let started = app.world().resource::<Time<Fixed>>().elapsed_secs_f64();
    for _ in 0..60 {
        app.update();
    }
    let elapsed = app.world().resource::<Time<Fixed>>().elapsed_secs_f64() - started;
    assert!(
        (elapsed - 1.0).abs() < 0.000_01,
        "60 accelerated updates must advance one simulated second, got {elapsed}"
    );
    assert!(
        app.should_exit().is_none(),
        "an ongoing battle must keep running"
    );
    assert_eq!(
        app.world_mut().query::<&Window>().iter(app.world()).count(),
        0
    );

    let opponent = app
        .world_mut()
        .query_filtered::<(Entity, &SwarmId), With<Swarm>>()
        .iter(app.world())
        .find(|(_, id)| !id.is_player())
        .map(|(entity, _)| entity)
        .expect("AI Battle startup must spawn the second swarm");
    let roots = app
        .world_mut()
        .query::<(Entity, Option<&SwarmMember>, Option<&OwnerSwarm>)>()
        .iter(app.world())
        .filter(|(entity, member, owner)| {
            (app.world().get::<Nanobot>(*entity).is_some()
                && member.is_some_and(|member| !member.0.is_player()))
                || owner.is_some_and(|owner| owner.0 == opponent)
        })
        .map(|(entity, _, _)| entity)
        .collect::<Vec<_>>();
    assert!(!roots.is_empty());
    for entity in roots {
        app.world_mut().despawn(entity);
    }
    app.update();
    assert_eq!(app.should_exit(), Some(AppExit::Success));
    let runs = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1);
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(runs[0].join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["status"], "completed");
    assert_eq!(summary["outcome"], "swarm_0_wins");
    assert_eq!(summary["seed"], 42);
    assert!(summary["latest"]["simulation_seconds"].as_f64().unwrap() > 1.0);
    assert!(
        fs::read_to_string(runs[0].join("samples.csv"))
            .unwrap()
            .lines()
            .count()
            >= 3
    );
}
