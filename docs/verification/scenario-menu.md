# Scenario menu verification

Issue: [#70](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/70).

ESC opens a paused menu with Standard and Sandbox selection, separate current/next-launch labels, Resume, and Quit. Sandbox retains the map's deposits, omits the opponent, and disables match outcomes. Runtime preferences survive process restarts; test app builders use a hermetic default.

## Automated checks

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passed. The final full test run passed 350 unit tests, 507 behavior tests, and 63 playtests, with two existing ignored playtests. A focused run passed all five scenario-menu playtests, including the existing-pause regression test, and the separate Sandbox behavior test passed.

`tests/playtest/scenario_menu.rs` covers immediate ESC pause, both resume routes, keyboard/camera input blocking and scroll draining, selection labels and persistence, Quit, menu actions after defeat through the real agent-button path, and existing-pause preservation. `tests/behavior/scenario_selection.rs` compares irrecoverable production in Standard versus Sandbox. Persistence unit tests cover missing/invalid settings, both selection roundtrips, failed-save preservation, and retry. The Sandbox collapse comparison was moved from playtest to behavior coverage; no behavioral protection was removed or consolidated. Enabling Sandbox outcome detection made the retained test fail with Defeat instead of InProgress both before and after relocation; it passed after restoring the implementation.

Representative fault checks failed as expected when successful saves confirmed the wrong scenario, opening-frame delta suppression was removed, terminal match gating rejected menu buttons, or closing the menu discarded an existing pause. Each mutation was restored and the focused tests passed afterward. The GPU suite also caught unconditional unpausing of screenshot fixtures; the menu now changes the clock only while it owns the pause and restores its prior state.

All 41 offscreen GPU trials passed across runs. The final `cargo test --test screenshots -- --ignored` completed 37 trials without failures, then stalled at `opponent_gameplay_loop`; that process was terminated. The four remaining trials passed in separate processes using `cargo test --test screenshots -- --ignored --exact NAME`: `opponent_gameplay_loop`, `regional_allocation`, `world_space_nanobots`, and `zone_binary_overlay`. A successful uninterrupted all-trial GPU batch remains unproven.

## Real-process checks

The rebuilt binary ran with `--headless --agent-socket`, a private mode-0700 runtime directory, isolated XDG configuration, and the repository asset root. All actions used `scripts/nano_swarm_control.py`; no desktop window or compositor input was used.

The external client verified Standard with one opponent, menu pause over explicit frame waits, rejection of map/intent/camera commands while paused, selection of Sandbox without changing the current Standard session, Quit, a fresh Sandbox with zero opponents, selection and relaunch of Standard, resumed fixed ticks, a forced save failure preserving Standard, invalid-setting fallback, and clean shutdown. Processes exited successfully; their sockets and temporary runtime/configuration directories were removed.

Inspected artifacts under `target/playtest-screenshots/`:

- `scenario_menu.png`: centered menu, dimmed game, readable current/next-launch labels, selected Sandbox border, Resume and Quit.
- `menu-runtime-standard-to-sandbox.png`: Current Standard and Next launch Sandbox over the loaded game scene.
- `menu-runtime-sandbox.png`: both labels Sandbox after a fresh process launch; opponent absence was independently checked through state.
- `menu-runtime-save-error.png`: inline retry guidance with both labels still Standard and the Standard selection retained.

The code-review skill ran independent Standards and Spec reviews against the staged changes from `f83cf8c366fab686e5fedefdac8f1ceca0c3d239`. Standards found an unbounded response wait and misplaced simulation coverage; both were corrected and re-reviewed. Non-blocking suggestions to separate shared input policy from menu presentation and consolidate protocol button dispatch were retained as optional design improvements. Spec found no missing or incorrect behavior. Gameplay window behavior was verified through scripted Bevy input and offscreen rendering, not an OS window.
