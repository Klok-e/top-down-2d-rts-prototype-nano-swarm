# Issue #65 production exit acceptance

Verified 2026-09-05 from `bec1fdb`. The existing production implementation already retains one funded output, counts it toward typed demand, excludes exit waiting from Production Pressure, and releases into body-clear exterior cells in oldest-completion order. This follow-up completes simulation and visual acceptance coverage without changing production behavior.

## Deterministic evidence

- `tests/playtest/production_exits.rs::both_swarms_release_paid_output_once_when_movement_opens_the_shared_exit` runs the real ProductionPlugin and movement systems. Player and opponent facilities consume twenty minerals each from initial balances of 20 and 33. Thirteen Workers block all exits for 250 updates. Moving one Worker opens a shared exterior cell; each released output moves away through navigation, allowing the other facility to release. All 400 subsequent updates check pairwise body separation and clearance from both facilities. Exactly one Defender and one Hauler emerge with their respective owners, and final buffers and ledgers remain 0 and 13.
- `tests/behavior/production_exits.rs::retained_output_satisfies_typed_demand_without_funding_a_duplicate` produces and retains a Defender for a one-tile reserve. A second facility with forty minerals remains idle for 250 updates: retained output covers demand while no Defender is present in the world.
- Existing production-exit tests retain coverage of oldest-output priority, clearing barriers, release after opening, and destruction without refund. Shared physical geometry tests cover structure and deposit clearance; this follow-up does not introduce another geometry implementation.

Both new tests were proved sensitive by temporarily changing their literal expected outcome, observing failure, and restoring the assertion. The screenshot author likewise checked a failing blocker-count assertion before restoring it. An initial demand fixture failure exposed a helper that automatically funded its facility; the final fixture creates and funds facilities explicitly, so expected balances are independent of helper defaults.

## Offscreen and runtime evidence

`screenshots/production_exits.rs::production_exit` replaces the former callback in `construction_lifecycle.rs`. It uses the full offscreen app, preserves the completed Hauler while all eight exterior cells are occupied, moves the south Worker through normal movement, and checks exact exit position, body separation, and no duplicate output across later frames and screenshot readback updates.

Inspected artifacts:

- `target/playtest-screenshots/production_exit_waiting.png`: eight green Workers surround the facility; the completed progress bar remains full, and no Hauler is outside.
- `target/playtest-screenshots/production_exit_released.png`: one yellow Hauler occupies the vacated south exit, with the Worker visibly farther south and separated from it.

Both independent reviewers also inspected these two images. They establish the scripted waiting/release scenario; the external client smoke run below establishes normal real-process wiring separately.

A fresh process ran with a private mode-0700 `XDG_RUNTIME_DIR` and `cargo run -- --headless --agent-socket --width 1280 --height 720`. Through `scripts/nano_swarm_control.py --socket /tmp/nano-swarm-issue65-runtime/nano-swarm/control.sock`, commands were `hello`, `paint defend 1 1`, `screenshot --name issue65-start`, `wait --fixed-ticks 300`, `screenshot --name issue65-after`, `state`, and `shutdown`. The inspected `target/issue-65-verification/start.png` and `after.png` show the authored facilities and separate nanobots, then exterior resource activity and Defenders advancing through the painted area. State at tick 326 reports player W4/H2/D3 and 104 minerals. Shutdown at tick 328 exited with status zero and removed the socket. An earlier run reached match completion while verification continued; it was shut down cleanly and restarted for this bounded flow. No desktop window or compositor automation was used.

## Checks and review

Passed commands:

```text
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test screenshots -- --ignored
git diff --check
```

The full default suite passed 363 unit tests, 514 behavior tests, and 46 scripted playtests; two GPU playtests and 37 screenshot trials were ignored in that run. All 37 offscreen screenshot trials passed separately. Focused tests passed after restoring assertion mutations. Logs, client responses, and runtime captures are under `target/issue-65-verification/`; generated artifacts are not committed.

Standards review found no hard violations; it noted an optional preference for an enum over numeric screenshot phases. Spec review found no missing requirements or scope creep in this acceptance follow-up. Performance acceptance at 5,000 nanobots remains issue #66.
