# Issue #60 navigation acceptance

Verified 2026-09-05 from `28226ec`, following the implementation in `dae230b` and the spatial lifecycle consolidation in `28226ec`. Issue #60's navigation behavior was already implemented; this follow-up adds two missing regression scenarios and records current acceptance evidence. It does not extend the scope to the #66 scale benchmark.

## Deterministic coverage

The public Navigation boundary exercises bounded pending searches, Clearing/Invalidated/Routine priority, aging under continuing urgent arrivals, stale-result rejection, reopening unreachable requests, and shared construction/search allowances. Existing simulation tests preserve Worker demand and defer placement while the budget is zero. `queued_movement_waits_safely_and_retries_after_an_obstacle_is_removed` covers a newly blocked active route with replacement work paused.

New scripted playtests in `tests/playtest/shared_navigation.rs`:

- `corridor_edits_preserve_active_travel_and_guide_the_next_leg`: an already-moving Hauler finishes its unpainted straight leg with the search budget stopped after owned Corridor paint changes. Its return leg visits the newly painted Corridor and arrives at the destination.
- `unrelated_obstacle_removal_preserves_travel_with_search_budget_stopped`: a moving Worker reaches its destination after an unrelated obstacle is removed, even with no allowance to compute another route.

Both tests use the approved simulation seam, explicit ticks, and observable positions. Temporarily inverting their key outcome assertions made exactly these two tests fail; the assertions were restored. Initial Corridor-fixture failures exposed an insufficient travel horizon and idle spreading after arrival; the final test checks the active leg and stops observing it upon completion.

The current simultaneous-replanning fixture again measured 24 completed requests in 269 advances at a cap of 1,000 work units per advance, with maximum reported request latency 270 ticks (including the initial pre-obstruction advance). The single-detour fixture took 146 advances at 100 units per advance. Logs: `target/issue-60-verification/navigation.log`.

The allowance meters cooperative search operations, including geometry predicates and lazy hierarchy rebuilding. Scheduler selection, sorting, snapshot copying, obstacle-diff/cache invalidation, and housekeeping have additional unmetered overhead. These counts are not total CPU instructions, wall-time bounds, or evidence of 5,000-Nanobot performance. Scale and tick-time measurements remain #66 work.

## Rendering and runtime

All 36 offscreen screenshot trials passed. The inspected `shared_navigation_before`, `shared_navigation_detour`, and `shared_navigation_arrived` captures show a Worker, Hauler, and Defender starting left of physical obstacles, passing around the rectangles with visible clearance, and finishing beyond the deposits.

A fresh real binary ran with `--headless --agent-socket --width 1280 --height 720`, a private mode-0700 runtime directory, and the repository control client. After painting Defend at (1, 1), the client waited 300 fixed ticks, read state, and captured a second frame. The inspected start/after captures show separate Nanobot bodies, exterior resource activity, and Defenders advancing into the painted area. The later frame shows 144 player minerals and population W4/H2/D3. Shutdown at fixed tick 336 exited successfully, removed the socket, and released its lock; the runtime directory was removed.

The initial restricted sandbox could not expose the GPU or bind test Unix sockets. The unrestricted rerun passed; no desktop window or compositor automation was used. Runtime commands, screenshots, test logs, and the temporary assertion-inversion log are under `target/issue-60-verification/` and are not committed.

## Final verification

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass: 363 unit tests, 496 behavior tests, and 40 scripted playtests. The default run skips 2 GPU playtests and 36 screenshot trials; the screenshot target was separately run with `--ignored` as described above.

Independent Standards review requested explicit player membership in the owned-Corridor fixture and stronger two-dimensional arrival assertions; both were applied. Spec review found no missing requirements or scope creep in this follow-up. No production changes were necessary.
