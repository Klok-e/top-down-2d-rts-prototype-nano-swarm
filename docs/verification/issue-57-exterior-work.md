# Exterior interaction acceptance — issue #57

Verified 2026-09-05 against [issue #57](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/57).

The shared interaction region uses circular deposit footprints (their declared world-space radius) and scaled, rotated rectangular structure footprints. A 34-unit body bound encloses all six rendered Nanobot silhouettes, including complete pixel extents; the widest Defender pixel corner is approximately 32.65 units from its sprite center. Work is permitted 34–38 units from a footprint surface. Movement approaches 36 units from the surface with a two-unit stopping tolerance. Local separation tuning and route planning are unchanged.

Assignment, arrival, and ongoing extraction, construction, maintenance, pickup, Worker unloading, Hauler delivery, and charging use this region. Displacement suspends effects and resumes approach while retaining task progress, reservations, and physical cargo.

## Deterministic evidence

- `tests/behavior/exterior_movement.rs`: actual movement to a scaled structure corner, rejection of interior and insufficient-clearance positions.
- `tests/behavior/exterior_work.rs`: gathering, Worker unloading, construction, maintenance, and charging; displacement/reapproach, scaled targets, rotation, ownership, and preserved resources.
- `tests/behavior/exterior_haul.rs`: both swarms picking up and delivering to scaled Stockpiles, Production Facilities, and Chargers; no remote transfer after displacement.
- `tests/playtest/exterior_work_flow.rs`: `exterior_gather_trip_extracts_and_delivers_at_scaled_target_surfaces` and `exterior_hauler_trip_loads_and_unloads_without_center_teleports`. Both run complete physical trips and measure body position whenever material moves.

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passed: 342 unit, 468 behavior, and 31 playtest tests. Two GPU playtests remain ignored by default; GPU screenshot validation runs separately.

## Offscreen evidence

`cargo test --test screenshots -- --ignored` passed all 28 trials. The run produced 52 PNG artifacts, inspected via contact sheets with the four new exterior captures also inspected individually. The new full-app trials assert actual extraction, delivery, or supplied charging before capture:

- `target/playtest-screenshots/exterior_work_gather.png`: Worker body and forward projection are clearly separated from the circular deposit.
- `target/playtest-screenshots/exterior_work_delivery.png`: Worker is fully outside the doubled-width Stockpile while its material indicator increases.
- `target/playtest-screenshots/exterior_hauler_delivery.png`: Hauler, including its pointed front, is outside the Production Facility during unloading.
- `target/playtest-screenshots/exterior_defender_charging.png`: Defender silhouette is outside the scaled Charger during a supplied Charge pulse.

A separate Spec reviewer independently inspected the captures. Its body-clearance finding was corrected; Standards and Spec reviews have no remaining findings.

## Real-process scenario

The binary ran with a private mode-0700 `XDG_RUNTIME_DIR` at `target/issue57-runtime`, using `cargo run -- --headless --agent-socket --width 1280 --height 720`. All external actions used `scripts/nano_swarm_control.py`:

1. Set camera to `(-256, 256)`, erase Gather at `(-1, 0)`, wait 60 fixed ticks, repaint Gather, and wait 240 fixed ticks.
2. Capture `exterior-final-gather` and query state. The inspected frame shows Workers outside the authored deposit beside a completed Source Stockpile, with material flow and active production visible.
3. Set camera to `(416, 256)`, wait 60 fixed ticks, and capture `exterior-final-base`. The inspected frame shows the live base, Hauler traffic, resource indicators, and combat presentation.
4. Send `shutdown`; verify process exit code 0 and removal of `control.sock` and `control.lock`.

Runtime captures and state are under `target/issue57-runtime/` and are not committed. No OS/compositor window was created. These runtime captures establish real application wiring; the deterministic tests establish exact work outcomes and position bounds. Global blockers, construction access preservation, and crowd collision resolution belong to later tickets.
