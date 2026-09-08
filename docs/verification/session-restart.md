# In-process scenario restart verification

The menu starts the confirmed scenario in the current application, including restarting the same scenario. Selection remains persistent. Escape opens and closes the menu; the on-screen menu launcher and Resume button are removed. Agent protocol v4 exposes `menu.toggle` and the real `menu.start` button.

## Automated coverage

`tests/playtest/session_restart.rs` covers the real Start button (`menu_start_replaces_the_match_and_keeps_the_application_shell`), removal of runtime-spawned entities and reset of minerals, intent, outcomes and camera (`restarting_clears_mutated_session_state_and_rebuilds_the_selected_scenario`), scenario-dependent painting/spectator UI (`intent_panel_tracks_the_scenario_across_same_process_restarts`), and interrupted/completed AI Battle recordings (`replacing_ai_battles_interrupts_active_runs_and_preserves_completed_runs`). `tests/playtest/headless_pacing.rs` verifies that headless time policy follows scenario changes. Existing menu playtests retain Escape pause/resume, input blocking, persistence, failed saves and completed-match controls; obsolete button actions were replaced without deleting their behavioral protection.

Failure proofs detected a deliberately omitted ledger reset, a disabled pacing update, and the missing spectator-panel rebuild. The production faults were restored and the tests passed afterward.

Formatting, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passed. The final default run passed 349 unit tests, 500 behavior tests and 73 playtests; three existing playtests remained ignored. All 45 offscreen checks passed individually in fresh processes after a batch run stalled. The affected scenario-menu and AI Battle screenshot checks were rerun after the final UI fix.

## Real-process proof

Run `cargo build`, then `python3 scripts/verify_session_restart.py --artifacts target/session-restart-live-proof-final` with an unused artifact directory. The script launches an offscreen process with isolated configuration/runtime directories, invokes the external control client, changes the camera before each restart, and exercises Standard → Sandbox → AI Battle → Standard → Standard. It verifies the scenario, swarm count, unfinished match, closed menu and reset zoom after every start. It also verifies AI recording interruption and clean process shutdown, socket removal and lock release.

The final run completed all four restarts in one process. Its responses, recording, log and inspected screenshots are under `target/session-restart-live-proof-final/`:

- `menu.png`: readable Current/Selected labels and Start button; neither removed button is present.
- `0-sandbox.png`: the menu is closed and the camera shows the fresh player start.
- `1-ai_battle.png`: two swarm HUD entries and “Spectating | Painting disabled”; no painting buttons remain.
- `2-standard.png` and `3-standard.png`: the player HUD and four painting controls return, and each restart returns the camera to the starting scene.

Verification created no desktop/compositor window.
