# Shared navigation — issue #59

Implemented against [#59](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/59) and the approved navigation/simulation/presentation seams in [#41](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/41). Verification date: 2026-09-05. **Acceptance remains incomplete: three existing scripted playtests fail.**

## Implementation

`Navigation` owns physical geometry, analytic swept-body clearance, fine-cell connectivity, 8×8-cell chunk regions and cross-chunk connections, detailed routes, explicit unreachable outcomes, and route costs. Fine cells remain 72 world units, with a 34-unit body radius. A reverse connectivity probe bounds disconnected-goal exploration when the destination is enclosed in a small component. Haulers sample owned/visible Corridor paint at a 0.35 multiplier; other types use ordinary distance.

Every movement order now names its final point or exterior interaction region. Gathering, construction, maintenance, pickup, delivery, charging, pursuit, staging, and procedural roaming use the shared boundary. Interaction routes may approach another face when the nearest face is blocked. Ranged pursuit can reach another accessible part of its attack region. The final integration step checks the whole displacement after separation and idle steering, so intermediate waypoints cannot conceal corner cutting. Planned Structures remain traversable; all completed structure kinds and existing Resource Deposits block both swarms. Depletion leaves the deposit solid; removing it reopens routes.

The Hauler-only planner, route component, follower, and direct-travel fallbacks are removed. Logistics rejects unreachable legs without assigning a finite reachable cost. Active routes survive paint changes and removal of unrelated blockers; the next obstructed segment triggers replanning. Waiting routes retry after geometry changes. Search budgeting, cargo recovery, crowd yielding, completion clearing, and production exits remain separate tickets.

The authored player seed position is `(220, 256)`: the snapped initial facility begins at x=288, so the former x=256 seed position had only 32 units of body clearance. Structure geometry and intent positions retain their authored values.

## Deterministic evidence

- Navigation public-boundary tests cover literal cross-chunk detours, disconnected regions inside one chunk that reconnect outside it, a sealed wall, a one-cell passage, diagonal corners, same-cell endpoints, Corridor cost/bias/ownership, and an inaccessible nearest interaction face.
- `tests/playtest/shared_navigation.rs` covers both swarms and all three types against completed Stockpiles, Chargers, Production Facilities, and deposits; independently samples each short segment against literal geometry; tests depletion/removal, traversable plans becoming blockers mid-route, reachable alternative gathering and pursuit approaches, and continued edge-cell roaming around an occupied center.
- The all-type movement regression failed against baseline `15a3736`: a Worker crossed into the wall's body-clearance bound at x=-105.5. The occupied-center roaming regression failed with an unchanged position over its final 100 ticks, then passed after the fallback advanced through free cells.
- Corridor tests compare two actual Hauler journeys: erasing paint preserves the already committed leg, while the following delivery leg reflects the changed paint. Both deliver 20 minerals. Assertion inversion confirmed that the trajectory assertion executes and fails.
- Existing fixtures that placed nanobots inside solids, overlapped independent logistics endpoints, or used destinations outside the finite world were corrected explicitly at their call sites. The ledger transport test now observes nonzero physical cargo/delivery and asserts source + cargo + sink conservation each tick.

Final commands used `CARGO_INCREMENTAL=0` after an incremental linker failure; no dependencies or broad build directories were removed.

| Check | Result |
| --- | --- |
| `cargo fmt` | Passed |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo test` unit target | 351 passed |
| `cargo test` behavior target | 469 passed |
| `cargo test` playtest target | 35 passed, 3 failed, 2 GPU tests ignored |
| `cargo test --test screenshots -- --ignored` | 30 passed |

Logs and diagnostic artifacts are in `target/issue59-verification/` (uncommitted).

## Offscreen and real-process evidence

The full screenshot suite produced 56 fresh PNG captures. Contact sheets were inspected for geometry, presentation, combat, exterior work, and staging. The three new `shared_navigation_{before,detour,arrived}.png` captures show distinct Worker, Hauler, and Defender bodies before their walls, passing wall corners with visible clearance, and arriving beyond the circular deposits. Their producing agent and the independent Spec reviewer inspected them. The staging captures retain physical 3/3 and 2/2/2 layouts with routed local roaming.

A real process ran with a private mode-0700 `XDG_RUNTIME_DIR` at `target/issue59-runtime`, using `cargo run -- --headless --agent-socket --width 1280 --height 720`. Only `scripts/nano_swarm_control.py` drove the session:

1. Set camera to `(512, 256)` and capture at fixed tick 835.
2. Paint Defend at `(1, 1)`, synchronize with 120 fixed ticks, and capture at tick 969.
3. Inspect both captures: Defenders visibly move across the right side of the completed facility toward expanded Defend paint; the facility remains fixed. The HUD mineral count increases from 248 to 284. At tick 975, both swarms remain in progress with three Defenders each.
4. Send `shutdown`; process exit code 0 and removal of `control.sock` and `control.lock` were verified.

Captures and responses are under `target/issue59-runtime/` and `target/issue59-verification/headless-control.jsonl`. These frames demonstrate real wiring and movement, not long-duration economy acceptance. Later live frames also expose the unresolved lifecycle/crowd limitations below. No OS/compositor window was created.

## Remaining failures and scope

These failures are retained, not ignored or relaxed:

- `defender_feel::authored_charger_planning_and_maintenance_follow_observed_service_need`: a Charger plan completes around its low-Charge Defender. The Defender is inside the new solid and cannot move to an exterior service point. Occupant clearing belongs to [#64](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/64).
- `defender_feel::authored_default_scenario_reaches_primary_defend_contest`: the long economy proof reaches Defeat. Diagnostics show three player Workers inside completed support geometry and multiple produced opponent Haulers at the solid facility center `(1620, 252)`. Completed-plan clearing and exterior production exits belong to #64 and [#65](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/65). These are observed hazards; the diagnostic snapshot alone does not isolate their individual contribution to the terminal result.
- `defender_feel::default_front_has_readable_combat_and_staggered_sustain`: three previously admitted charging rotations persist after casualties reduce six living Defenders to five. The existing selector caps new admissions but does not reconcile active rotations after casualties; the test requires a continuous cap. Changed movement timing exposes this separate Charge lifecycle gap.

The two-axis review found no confirmed code defects in the reviewed routing changes. Standards noted nonblocking endpoint-identity reconstruction in the standalone Hauler assignment adapter and repeated route-cost parameters. The reported Corridor-stability and blocked-roaming coverage gaps were corrected. Spec review explicitly retains the failed full-suite gate as an acceptance limitation. Issue #59 remains open pending that gate and the scoped dependency decision.
