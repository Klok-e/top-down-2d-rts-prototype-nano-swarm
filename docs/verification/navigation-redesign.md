# Responsive navigation and continuous movement

Implements the accepted decisions in [ADR-0016](../adr/0016-shared-hierarchical-navigation.md). The prior local-approach changes and their initial measurements remain documented in [movement-approaches.md](movement-approaches.md).

## Changes

Work assignment now asks for geometric connectivity and ranks reachable goals by estimated distance. Connectivity uses the fine grid's exact connected components, reuses their adjacency, and does not materialize disposable detailed routes. The obsolete speculative detailed-route query API was removed. Loaded deliveries keep valid destinations and reservations despite cheaper alternatives or pending access checks.

Actual movement owns a cancellable route request. It receives three quarters of the search budget; background connectivity and construction receive one quarter. Both can borrow unused capacity, and individual searches advance in 128-work slices. Diagnostics report request purpose and per-owner pending age. Haulers without owned Corridor paint avoid unnecessary cost optimization; with Corridor paint, the approved clear final approach within 144 units bypasses optimization.

Routes retain validated safe prefixes during replanning and join replacement results from actual progress. Selected work positions no longer require one full search for selection followed by another for movement. Geometry and clearing barriers remain authoritative, including for direct approaches.

The local controller retains actual velocity and uses responsive acceleration, braking, and turning with swept collision checks. Yielding ends when normal movement is safe again. Its admission/release decisions use actual movement steps rather than extrapolating beyond route corners or arrival points.

Evacuation selects separate exit space and ends once the bot is physically clear, even if construction validation is pending. It preserves unrelated movement orders; construction still waits for its required access check and vacancy.

## Regression evidence

- `tests/playtest/defender_feel.rs`: `default_economy_loaded_bots_do_not_wait_for_clear_routes` runs both authored economies for 60 simulated seconds at the configured speed of 5 units per tick and 60 Hz. It requires exercised loaded travel, no clear unblocked movement pause beyond 15 ticks (250 ms), and no movement request older than 15 ticks. The initial reproduction, using the existing helper's speed of 5.25, failed at 177 ticks. Final verification uses the actual game speed explicitly.
- `tests/behavior/work_navigation.rs`: `clear_final_hauler_approach_moves_without_search_capacity` starts a clear approach on the first movement tick and reaches standing space with zero search budget, preserving cargo.
- `tests/behavior/structure_clearing.rs`: `evacuated_bot_resumes_work_while_construction_validation_is_pending` failed before the lifecycle fix and passes afterward. It retains pending construction, releases evacuation, resumes a distinct order, and preserves cargo at zero budget.
- `tests/behavior/worker_route_recovery.rs`: `loaded_delivery_revalidation_keeps_commitments_without_materializing_routes` preserves worker and hauler destinations, reservations, and cargo while proving no movement requests or fine route refinement are needed for access checks.
- Navigation budget regressions cover bounded service for movement and background work, per-request fairness, exact connectivity across detours and dividing walls, and preservation of construction access protections. The scheduler regression failed against the previous head-of-line scheduling behavior.
- Local avoidance regressions cover gradual starts and turns, endpoint braking, stationary-hostile passing without displacement or overlap, and progress along a safe route prefix while replacement search has zero budget. The smooth-start regression failed against the previous immediate full-speed movement.

The existing crowded-economy scenarios still prove 11-worker delivery, concurrent 12-hauler initial and repeat delivery, twenty separate hauler unloads, and terminal and construction access. Corridor movement tests remain; the assignment test now expects the approved estimated-cost ranking rather than full-route optimization.

## Checks and visual inspection

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passed: 374 unit tests, 543 behavior tests, and 59 playtests; two playtests remain ignored. All 41 offscreen tests passed with `cargo test --test screenshots -- --ignored`.

Inspected the three refreshed `approach_delivery` captures in `target/playtest-screenshots/`. All eleven bots are visible in the distant line, advance toward the stockpile, then occupy separate positions around it. There is no obvious clipping or rendered overlap. These frames establish presentation and scene coverage; movement timing is established by the simulation assertions.

Independent read-only reviews found no correctness issues in the scheduler, connectivity, owned routes, controller, or evacuation changes. Remaining code quality opportunities are typed connectivity cache keys and a distinct internal connectivity result instead of the scheduler's shared route-result representation. No desktop/compositor window was used.

## Scaling measurement

Ran `NAV_BENCH_OUTPUT=target/navigation-redesign/scale cargo bench --bench navigation_acceptance` for 5,000 bots over 600 ticks per scenario. All geometry and per-tick work-budget assertions passed. This benchmark excludes allocation, economy, and rendering.

| Scenario | Tick p95 (ms) | Bots moved at least once | Requests pending at end |
| --- | ---: | ---: | ---: |
| Open | 41.025 | 5,000 | 0 |
| Obstacle dense | 81.149 | 1,800 | 3,200 |
| Bottleneck | 11.205 | 100 | 4,900 |
| Simultaneous replanning | 50.201 | 5,000 | 5,000 |

Large-scene CPU cost remains a material limitation. These timings are higher in three scenarios than the earlier local-approach measurement (open 15.011 ms, dense 18.828 ms, replanning 14.456 ms). More bots begin moving in the dense case, but the measurements do not isolate the causes of the CPU increase. This is not a controlled performance attribution or a 5,000-bot smoothness claim. Full data: `target/navigation-redesign/scale/report.json`.

## Real-process observation

Ran `cargo run --example movement_diagnostics`, which builds the unmodified runtime with headless rendering and the agent socket, using the actual game configuration. The external `scripts/nano_swarm_control.py` client captured state, waited for explicit fixed ticks, captured before/running frames, and requested shutdown. No scene or simulation-clock mutation was used.

The run covered 99 simulated seconds:

- Longest observed stationary interval attributed to an owned pending route: **0.033 seconds**.
- Longest observed loaded-bot stationary interval of any cause: **1.183 seconds**. The only two periodic snapshots above one second were the same worker waiting for destination standing space. It subsequently resumed; this was not a route availability delay.
- Background request counts were transiently large (peak sampled total pending 454), but active movement continued. The final sample still had 90 background requests (83 connectivity, seven construction) and no movement requests. The oldest background request was 2,301 ticks old, so expensive background planning remains a limitation even though it no longer blocks movement. Counts are sampled diagnostics, not a complete request trace; this evolving construction scene does not establish steady-state queue drainage.
- Player minerals rose from 155 in the early state capture to 571; opponent minerals rose from 156 to 528. Player population changed from W4/H2/D3 to W5/H11/D3; opponent from W4/H3/D3 to W6/H10/D2. These are observed scenario outcomes, not normalized throughput benchmarks.

Inspected both real-process captures: workers and haulers occupy exterior work areas, the final frame shows a populated transport route and additional support construction, and the terrain and UI render correctly. Static frames cannot establish smoothness; the continuous-controller regressions and temporal observation supply that evidence. The diagnostic's direction-reversal counter is not normalized for increased population or travel and is not used to claim a quantitative smoothness improvement.

Evidence: `target/navigation-redesign/runtime/` contains logs, state snapshots, control responses, inspected images, and `summary.json`. The process exited successfully; socket disappearance and exclusive reacquisition of the lifecycle lock were verified before removing the private runtime directory.
