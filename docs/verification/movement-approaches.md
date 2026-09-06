# Local work approaches and route following

Implemented the approved sequence: approach a destination before claiming standing space, smooth retained routes, then improve local passing. Navigation budgets and the global routing algorithm are unchanged.

## Behavior

- Workers and haulers travel toward statically reachable work positions without reserving distant standing space. Within 144 world units they search locally; they remain in local search until more than 216 units away. Existing approach age determines priority, including after yielding.
- Idle occupants leave demanded work areas. Loaded arrivals and assigned workers, including planned-structure builders, remain in place until work can run.
- Route followers look ahead at most eight waypoints and advance from actual position. Haulers only skip collinear waypoints, preserving Corridor turns. Shortcuts obey both physical obstacles and clearing entry barriers.
- Local avoidance tries half and quarter forward steps before passing, retains a passing side, and recovers when a returning yield trail becomes obstructed.

## Deterministic and rendered verification

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passed: 368 unit tests, 537 behavior tests, and 58 playtests (two ignored). `cargo test --test screenshots -- --ignored` passed all 41 offscreen tests. No compositor windows were created.

Added playtests in `tests/playtest/congested_economy.rs`:

- `loaded_workers_deliver_without_remote_waiting_or_removing_unloaded_workers`: all eleven workers deliver their four minerals within twenty simulated seconds; all workers remain alive and cargo plus stockpile contents remains 44 each tick. The initial reproduction stalled at 28/44 before the fixes.
- `loaded_haulers_approach_before_waiting_and_each_complete_delivery`: twelve haulers approach before waiting; every initial load is delivered within twenty seconds, then repeated trips empty the source within sixty seconds. The test checks conservation throughout.

Behavior tests cover occupied distant goals, slower following, friendly idle departure, and arriving work assignments. The planned-builder arrival test failed before adding its arrival protection. Seven route-follower unit tests cover speed, rejected motion, lookahead bounds, turns, and obstacle changes. The existing clearing-footprint detour test caught a shortcut regression and passes with the clearing-aware segment check.

Inspected `target/playtest-screenshots/approach_delivery_{before,searching,all_delivered}.png`. All eleven workers are visible: first in a distant line, then approaching, then dispersed around the stockpile. An independent reviewer inspected the captures and reviewed the implementation without findings. Static images establish scene presentation, not motion smoothness; the simulation checks establish delivery.

## Scale measurement

Ran `NAV_BENCH_OUTPUT=target/movement-verification/scale cargo bench --bench navigation_acceptance` with 5,000 bots for 600 ticks per scenario. Work stayed within 32,768 units per tick and obstacle-crossing assertions passed.

| Scenario | Tick p95 (ms) | Bots moved at least once | Requests pending at end |
| --- | ---: | ---: | ---: |
| Open | 15.011 | 5,000 | 0 |
| Obstacle dense | 18.828 | 340 | 4,687 |
| Bottleneck | 13.169 | 113 | 4,955 |
| Simultaneous replanning | 14.456 | 5,000 | 4,906 |

These are minimal movement/navigation measurements, excluding economy and rendering. They do not establish a frame-time guarantee or smooth movement for 5,000 bots in obstructed scenes. No matched baseline timing comparison was run. Full measurements are in `target/movement-verification/scale/report.json`.

## Real process observation

`examples/movement_diagnostics.rs` starts the unmodified default scene with headless rendering and the agent socket, observing actual loaded-bot positions. The external controller captured state and screenshots, then requested shutdown. The process exited, its socket disappeared, and its lifecycle lock was released; the private runtime directory was removed after retaining evidence.

The first run covered approximately 98 simulated seconds. Both swarms extracted minerals. The inspected before/running captures show workers and haulers around the exterior stockpiles, rendered terrain, and intact UI. Loaded-bot pauses still occurred, with a maximum observed interval of 6.433 seconds. This observation does not justify claiming that all default-map pauses are resolved. Evidence is under `target/movement-verification/runtime/`.

A second run added route/work-state diagnostics. Around 31–32 simulated seconds, stalled loaded workers and haulers had movement orders but no `RemainingTravel`, zero velocity, and no loading, extraction, yielding, work blockage, or congestion recovery. Navigation spent its full 32,768-unit budget with 154–175 pending requests, including while some stalled bots had statically clear direct goals. This points to delayed route publication as a remaining bottleneck. The local approach/following changes do not resolve global route-queue saturation. The diagnostic is observational evidence, not a controlled proof of scheduler cause. The second run continued through 68 simulated seconds: maximum observed stop 6.250 seconds, peak pending requests 526. All 107 periodic stalled snapshots lacked `RemainingTravel`; none were loading, two were extracting, two were work blocked, and none were recovering. Detailed evidence is in `target/movement-verification/detailed-runtime.log` and `target/movement-verification/detailed-runtime-process.json`. Its process also exited cleanly; socket removal and released lifecycle lock were verified before removing the private directory.

## Test consolidation

Kept the twelve-hauler congestion case and removed its eight-hauler variant, which exercised the same contract. Removed an assertion on the twenty-delivery test's own milestone counter; observable unload count and mineral totals remain. The screenshot keeps scene-presence, capture-deadline, and completed-frame checks. Per-tick physical conservation remains in the fast worker playtest, and its ledger assertion was moved there from the screenshot.

Temporarily introduced an extra mineral on each worker/hauler stockpile delivery. Before and after consolidation, the worker, concurrent-hauler, and single-hauler repeated-delivery tests rejected this accounting defect. Production files were restored byte-for-byte before normal verification. This checks retained accounting coverage; it is not an exhaustive mutation analysis of all assertions.
