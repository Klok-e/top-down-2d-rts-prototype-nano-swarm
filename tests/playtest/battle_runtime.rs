//! Real-process AI Battle playtests: the process serves its final statistics over the agent
//! socket after the tick limit, writes no files, and exits only on `shutdown`.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_TICKS: u64 = 120;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

/// A running battle process, its private runtime/config/working directories, and the control client.
struct BattleProcess {
    child: Child,
    root: PathBuf,
    socket: PathBuf,
}

impl BattleProcess {
    fn spawn(arguments: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nsb-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                % 1_000_000
        ));
        for directory in ["run", "config", "cwd"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let child = Command::new(env!("CARGO_BIN_EXE_top-down-2d-rts-prototype-nano-swarm"))
            .args(arguments)
            .args(["--agent-socket", "--scenario", "ai-battle", "--seed", "42"])
            .args(["--max-ticks", &MAX_TICKS.to_string()])
            .current_dir(root.join("cwd"))
            .env("XDG_RUNTIME_DIR", root.join("run"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("BEVY_ASSET_ROOT", env!("CARGO_MANIFEST_DIR"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the battle binary starts");
        let socket = root.join("run/nano-swarm/control.sock");
        let mut process = Self {
            child,
            root,
            socket,
        };
        let started = Instant::now();
        while !process.socket.exists() {
            assert!(
                process.child.try_wait().unwrap().is_none(),
                "the process exited during startup"
            );
            assert!(started.elapsed() < STARTUP_TIMEOUT, "startup timed out");
            std::thread::sleep(Duration::from_millis(50));
        }
        process
    }

    fn command(&self, arguments: &[&str]) -> serde_json::Value {
        let output = Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/nano_swarm_control.py"))
            .arg("--socket")
            .arg(&self.socket)
            .args(["--timeout", "60"])
            .args(arguments)
            .output()
            .expect("python3 runs the control client");
        let response: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("unreadable client reply ({error}): {output:?}"));
        assert_eq!(response["ok"], true, "{arguments:?}: {response}");
        response["result"].clone()
    }

    fn wait_for_exit(&mut self) -> std::process::ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the process ignored shutdown"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for BattleProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn final_statistics(process: &BattleProcess) -> serde_json::Value {
    let started = Instant::now();
    loop {
        let stats = process.command(&["battle-stats"]);
        if stats["summary"]["status"] != "in_progress" {
            return stats;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "the battle never reached its tick limit"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_serves_final_statistics_until_shutdown(arguments: &[&str]) {
    let mut process = BattleProcess::spawn(arguments);
    let stats = final_statistics(&process);
    assert_eq!(stats["summary"]["status"], "interrupted");
    assert_eq!(stats["summary"]["termination_reason"], "tick_limit");
    assert_eq!(stats["summary"]["seed"], 42);
    let last = stats["samples"].as_array().unwrap().last().unwrap();
    for swarm in last["swarms"].as_array().unwrap() {
        assert_eq!(swarm["fixed_tick"], MAX_TICKS);
        assert!(swarm["controller_reviews"].as_u64().unwrap() > 0);
    }

    process.command(&["wait", "--frames", "30"]);
    assert!(
        process.child.try_wait().unwrap().is_none(),
        "a finished battle must wait for shutdown"
    );
    assert_eq!(process.command(&["battle-stats"]), stats);
    assert_eq!(
        std::fs::read_dir(process.root.join("cwd")).unwrap().count(),
        0,
        "an AI Battle writes no files"
    );

    process.command(&["shutdown"]);
    assert!(process.wait_for_exit().success());
}

#[test]
fn simulation_only_battle_serves_final_statistics_after_the_tick_limit_until_shutdown() {
    assert_serves_final_statistics_until_shutdown(&["--simulation-only"]);
}

#[test]
#[ignore = "requires a GPU adapter; run with cargo test --test playtest headless_battle_serves -- --ignored"]
fn headless_battle_serves_final_statistics_after_the_tick_limit_until_shutdown() {
    assert_serves_final_statistics_until_shutdown(&[
        "--headless",
        "--unbounded",
        "--width",
        "320",
        "--height",
        "180",
    ]);
}

#[test]
fn battle_without_the_agent_socket_is_rejected_at_startup_without_files() {
    let root = std::env::temp_dir().join(format!("nsb-reject-{}", std::process::id()));
    for directory in ["run", "config", "cwd"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_top-down-2d-rts-prototype-nano-swarm"))
        .args(["--simulation-only", "--scenario", "ai-battle"])
        .current_dir(root.join("cwd"))
        .env("XDG_RUNTIME_DIR", root.join("run"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("BEVY_ASSET_ROOT", env!("CARGO_MANIFEST_DIR"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("the battle binary starts");
    let files = |directory: &str| std::fs::read_dir(root.join(directory)).unwrap().count();
    let (run, config, cwd) = (files("run"), files("config"), files("cwd"));
    let _ = std::fs::remove_dir_all(&root);
    assert!(!output.status.success(), "startup must be rejected");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("agent-socket"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!((run, config, cwd), (0, 0, 0), "no files are created");
}
