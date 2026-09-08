# AI Battle verification — issue #71

The scenario uses the authored Standard map and equal starting economies with two timed intent controllers. Scenario definitions select shared session policies; input, HUD, elimination, and recording do not branch on scenario identity. Match outcomes carry the actual winning SwarmId, and recording retains first elimination times by swarm identity.

## Automated evidence

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `git diff --check` passed.
- `cargo test` passed: 346 unit tests, 500 behavior tests, and 68 scripted playtests. Three GPU-bearing playtests and the 45 screenshot trials remain ignored by the default command.
- `cargo test --test screenshots -- --ignored` passed all 45 offscreen trials.
- `cargo test --test playtest accelerated_battle_runtime -- --ignored` passed. The full offscreen app advanced exactly one simulated second in 60 unpaced updates, then a controlled elimination fixture verified final files existed before successful AppExit.
- All eight recorder integration tests passed, covering sampling, gross event retention, final partial intervals, frozen results, interruption, arbitrary session policy without scenario selection, frame sampling above fixed-tick frequency, failed atomic summary replacement, and actual swarm IDs under different outcome perspectives.

Failure proofs covered unsupported launch options, a missing second controller, spectator input escaping its guard, and missing event recording. Disabling the nine producer instrumentation paths made all nine focused accounting tests fail; restoring them passed. Recorder faults that forced default outcome wording, restricted recording to a scenario name, or suppressed persistence errors made four recorder tests fail. All temporary faults were removed before the final passing suite. No tests were deleted or consolidated.

## Real process and inspected visuals

`python scripts/verify_ai_battle.py --artifacts target/ai-battle-proof-final` launched the actual binary with offscreen rendering, isolated runtime/config directories, the agent socket, seed 42, and a separate output directory. The client observed AI Battle, waited for fixed ticks, confirmed painting rejection, captured a screenshot, and sent SIGINT. The process saved three samples (six per-swarm CSV rows), with status `interrupted`, no invented outcome, and 2.600000052 simulated seconds; it exited with code 130, removed its socket, and released its lifecycle lock.

Inspected `target/ai-battle-proof-final/ai-battle.png`: the complete symmetric map and opposite starts are visible; both HUD entries show four Workers, two Haulers, three Defenders, and one facility. The generic spectator/painting-disabled label is visible, with no painting toolbar. Inspected `target/playtest-screenshots/scenario_menu.png`: all three scenario choices fit cleanly and the current/next-launch state is readable. Inspected `target/playtest-screenshots/match_defeat.png`: Standard retains its player-relative defeat banner after the internal outcome migration.

The real-process run proves launch, recording, spectator controls, interruption, and cleanup. Automatic terminal persistence/exit is proven through the controlled full-runtime fixture; this evidence does not claim that an unmodified natural battle reached elimination. No performance threshold or deterministic outcome guarantee is asserted. Artifacts remain under ignored `target/` directories; source verification helpers are committed.

The restricted sandbox initially blocked Unix socket tests and stalled audio initialization. Those validation processes were stopped and the affected checks passed with local socket/audio access; no automated check created a desktop window.
