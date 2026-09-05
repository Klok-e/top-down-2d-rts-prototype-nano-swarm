# Issue #64 structure clearing and cancellation

Acceptance follow-up from `0366b6c`, verified 2026-09-05. The existing lifecycle owns final access validation, entry barriers, evacuation, activation, and unsafe-plan cancellation. This change completes cross-system coverage and corrects evacuation toward neighboring clearing sites.

## Deterministic evidence

- `tests/playtest/adjacent_structure_clearing.rs::adjacent_structure_clearing_uses_free_exit_beside_congested_site` exercises adjacent completing sites through real navigation and local movement. A congested neighbor must not prevent the other site using its free exit. Every movement step checks body separation, bounded displacement, and swept clearance from an adjacent solid.
- `tests/playtest/construction_cancellation.rs::cancelled_facility_restarts_pressure_then_plans_one_alternative` forces actual final-check cancellation of a Production Facility. In the cancellation tick there is no operational capacity, pressure is zero, the fading effect reserves no footprint, and the site remains excluded. The next 59 ticks accumulate fresh pressure without a replacement; continued demand then creates and retains exactly one plan elsewhere.
- `tests/playtest/construction_cancellation.rs::worker_finishing_an_unsafe_plan_is_released_in_the_cancellation_tick` starts with a claimed Worker and one construction tick remaining. A new blocker makes completion unsafe. Real work and cancellation remove both Worker claim and progress while creating one visual effect.
- Existing `tests/behavior/structure_clearing.rs` covers evacuation of either swarm, blocked admission, waiting under congestion, changed-access cancellation, navigation refresh in the committing tick, simultaneous exclusions, and pending validation. Construction-access tests cover combined-plan friendly connectivity and alternate-site selection.

The cancellation tests were independently checked by changing literal expected pressure/effect-count outcomes, observing failures, and restoring the assertions.

## Visual evidence

`screenshots/construction_lifecycle.rs` pauses simulation for capture, lets a prepared fixed tick settle before cancellation snapshots, and verifies that captured scale and opacity survive readback. The full offscreen app still renders and exports the images.

The producing agent inspected all five fresh lifecycle artifacts under `target/playtest-screenshots/`:

- `construction_clearing_occupied.png`: a blue Defender occupies the yellow planned outline.
- `construction_clearing_activated.png`: the Stockpile is green and operational, with the Defender outside its lower edge.
- `construction_cancellation_pulse.png`: a full-size bright red outline surrounds the former site.
- `construction_cancellation_collapse.png`: the red outline is smaller and dimmer.
- `construction_cancellation_fade.png`: only faint small red corners remain behind the Defender.

These captures demonstrate occupied completion and the three cancellation phases without text or a desktop window. The parent also inspected pulse and fade captures.

## Regression and final verification

The adjacent-site playtest failed against the original evacuation checks after 180 updates: the upper site never activated despite free exits. With the fix, the same playtest passes. Both newly selected and retained goals now use `PhysicalWorld` occupancy and swept-movement checks, so another clearing site's entry barrier cannot become a permanently unreachable evacuation destination. The existing shared geometry remains authoritative.

Passed commands:

```text
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test screenshots -- --ignored
git diff --check
```

The full default suite passed 363 unit tests, 514 behavior tests, and 49 scripted playtests. Two GPU playtests and 37 screenshot trials were ignored by default; all 37 offscreen screenshot trials passed separately. Logs and generated runtime artifacts are under `target/issue-64-verification/` and are not committed.

A fresh `cargo run -- --headless --agent-socket --width 1280 --height 720` used a private mode-0700 runtime directory. The external `scripts/nano_swarm_control.py` client set the camera, read state, captured the start, painted Defend at (1, 1), waited 300 fixed ticks, read state, captured again, and shut down. At tick 323 the match remained in progress with player W4/H2/D3 and 40 minerals. Shutdown at tick 333 exited zero, removed the socket, released its lock, and allowed private-directory cleanup.

Inspected `issue64_start-434699-000000.png` and `issue64_after_paint-434699-000001.png` show separated starting bodies and a planned Stockpile, then completed Stockpiles, exterior gathering/transport, and Defender movement into contested paint. This is real-process wiring and ordinary construction evidence; forced cancellation and neighboring-site congestion are established by the deterministic playtests and full-app offscreen scenarios above.

Standards review found no documented-standard violations, with one optional suggestion to replace numeric screenshot phases and independent capture fields with an explicit state enum. Spec review found no missing requirements or scope creep and independently inspected all five lifecycle images. Neither review found a correctness defect in the final change. Scale/performance acceptance remains issue #66.
