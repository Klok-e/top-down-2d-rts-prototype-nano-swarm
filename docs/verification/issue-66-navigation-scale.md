# Navigation scale and aggregate acceptance

Scope: [issue #66](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/66),
under the accepted navigation specification in #41. Starting revision:
`ef1360e88e90562a447cd9ae24ce508535ab5bf9`. All three declared blockers
(#61, #64, #65) were closed when this task was selected on 2026-09-05.

## Aggregate behavior coverage

The aggregate uses the approved simulation, public Navigation, and full-app offscreen
seams. The scenarios below exercise connected system behavior; their narrower
assertions complement the real-process economy and traffic runs.

| Contract | Deterministic coverage | Offscreen capture |
| --- | --- | --- |
| Exterior gathering and physical delivery | `exterior_work_flow::{exterior_gather_trip_extracts_and_delivers_at_scaled_target_surfaces, exterior_hauler_trip_loads_and_unloads_without_center_teleports}` | `exterior_work` |
| Owned Corridor preference and stable active travel | `shared_navigation::corridor_edits_preserve_active_travel_and_guide_the_next_leg` | `physical_logistics` |
| All types and swarms respect obstacles | `shared_navigation::both_swarms_and_all_types_detour_around_completed_structure` | `shared_navigation` |
| Completion obstructs an active route before entry | `shared_navigation::planned_structure_allows_crossing_but_completion_replans_before_entry` | `construction_clearing` |
| Pending work preserves demand and placement decisions | `navigation_budget::{pending_worker_access_preserves_population_demand_until_budget_allows_assignment, pending_placement_waits_for_shared_navigation_work_before_reserving_a_plan}` | `shared_navigation` (rendering only) |
| Local yielding and retained routes through a narrow passage | `local_avoidance::following_traffic_yields_with_the_front_bot_in_a_bottleneck` | `local_avoidance`; real-process traffic fixture |
| Joint-plan friendly connectivity and builder access | `construction_access` behavior module | `construction_access` |
| Clearing cooperates with adjacent congested construction | `adjacent_structure_clearing::adjacent_structure_clearing_uses_free_exit_beside_congested_site` | `construction_clearing` |
| Cancellation releases claims and restarts pressure at another site | `construction_cancellation::{worker_finishing_an_unsafe_plan_is_released_in_the_cancellation_tick, cancelled_facility_restarts_pressure_then_plans_one_alternative}` | `construction_cancellation_{pulse,collapse,fade}` |
| Cargo waits through failed access, then transfers exactly once | `physical_logistics_flow::blocked_delivery_waits_with_cargo_then_reopens_without_duplicate_transfer` | `route_recovery_{waiting,delivered}` |
| Funded output waits for a free exit without duplicate cost | `production_exits::both_swarms_release_paid_output_once_when_movement_opens_the_shared_exit` | `production_exit_{waiting,released}` |

Names before `::` identify modules under `tests/playtest/` except the explicitly
identified behavior modules. Screenshots prove presentation; simulation assertions
prove custody, priority, clearance, and lifecycle outcomes.

## Measurement host

- Intel Core i5-13600KF, 14 cores / 20 logical CPUs, x86_64.
- Linux `7.1.9-zen1-2-zen`.
- `rustc 1.98.0-nightly (f428d123a 2026-06-19)`;
  `cargo 1.98.0-nightly (598ab48ec 2026-06-17)`.
- Cell width 72 world units; body radius 34. These preserve the accepted
  one-cell passage clearance. Intent cells remain independent of navigation cells.

## Inspected offscreen evidence

The fresh `cargo test --test screenshots -- --ignored` run passed all 37 trials.
Logs: `target/issue-66/screenshots.log` and `screenshots-final.log`. The primary agent opened all 22
navigation/lifecycle-related PNGs listed below under `target/playtest-screenshots/`:

- `shared_navigation_{before,detour,arrived}`: the Worker, Hauler, and Defender
  begin left of rectangles, pass their corners with visible clearance, and finish
  beyond the circular deposits.
- `local_avoidance_{before,yield,arrived}`: four distinct bodies start in the
  narrow passage; one backs out above the right wall end; the final view has two
  bodies at each side's destination markers and an empty passage.
- `construction_access_{open_passage,safe_alternative}`: the opening between
  wall sections stays clear, while the new plan appears to the lower left.
- `construction_clearing_{occupied,activated}`: a Defender starts inside a
  yellow planned outline, then stands outside the lower edge of the completed
  green Stockpile.
- `construction_cancellation_{pulse,collapse,fade}`: the full bright red outline
  shrinks and dims to small red corners behind the Defender, without text.
- `route_recovery_{waiting,delivered}`: a loaded Hauler waits left of the
  full-height wall, then stands beside the destination after that wall is removed.
- `production_exit_{waiting,released}`: eight Workers surround the facility;
  the south Worker moves out, leaving one yellow Hauler in the vacated exit.
- `exterior_work_{gather,delivery}`, `exterior_defender_charging`, and
  `exterior_hauler_delivery`: the working bodies remain outside the deposit,
  Stockpile, Charger, and terminal footprints.
- `physical_logistics`: separate Worker/Hauler bodies stand above the four
  distinct resource buffers, with visible cargo and local buffer indicators.

These visual facts satisfy the presentation portions of the aggregate acceptance.
They do not measure throughput or substitute for deterministic mineral balances,
route-retention assertions, or scale measurements.

## Reproducible scale workloads

```bash
NAV_BENCH_OUTPUT=target/issue-66/scale-final cargo bench --bench navigation_acceptance
```

`NAV_BENCH_TICKS` overrides the default 600 updates; `NAV_BENCH_OUTPUT` changes
the output directory (default `target/issue-66/scale`). The benchmark writes
`report.json` and per-scenario JSON containing every measured tick. The run log
is `target/issue-66/scale-final.log`; final JSON is under `target/issue-66/scale-final/`. It uses the bench profile and the shared
minimal simulation builder with real navigation and movement systems, 5,000
Workers, a 64-by-64 intent grid (32,768 world units square), and explicit 60 Hz
time. Allocation, economy, and rendering are excluded from these timings;
their combined wiring is verified separately in the real-process runs below.

The deterministic formation uses 72-unit column spacing, 144-unit row spacing,
x coordinates from -7,380 to -252, and front-first request order. Destinations
are 15,000 units east. Scenarios are open space, 56 rectangular obstacles, a
72-unit gap between two walls, and a wall inserted at x=1,008 after 30 ticks.
The replanning fixture first asserts that all 5,000 bodies have begun moving.
Every subsequent movement segment and position is checked against physical
navigation clearance. Timing covers `app.update()`, including navigation
snapshot refresh and scheduler overhead; external assertions/JSON encoding
are outside the timed interval.

| Scenario | Tick p50 / p95 / p99 (ms) | Completed searches | Pending at end | Largest observed completion latency (ticks) | Bodies beyond x=72 |
| --- | --- | ---: | ---: | ---: | ---: |
| Open | 3.552 / 7.421 / 9.709 | 5,000 | 0 | 1 | 1,850 |
| Obstacle dense | 8.055 / 11.580 / 14.961 | 23 | 4,977 | 595 | 2 |
| Bottleneck | 5.220 / 10.377 / 13.036 | 278 | 4,821 | 595 | 11 |
| Simultaneous replanning | 4.655 / 11.080 / 13.913 | 5,033 | 4,967 | 559 | 1 |

The replanning completion count includes 5,000 initial routes and 33 replacement
searches. Search completions are not unique bodies: local yielding can create
additional route work. The x=72 count establishes passage crossings in the
bottleneck; it is not a count of crossings of the x=1,008 replanning wall.
Bodies that moved at least once numbered 5,000 / 227 / 499 / 5,000 respectively;
local separation can move pending bodies, so that metric is not route completion.

Queues are heavily backlogged in all three obstructed cases. Pending latency
is censored by the ten-second simulation window; the table reports actual
completion maxima, not inferred per-request latency percentiles or an eventual
all-5,000 completion claim. There is no agreed frame-time/queue-latency guarantee.

| Scenario | Cooperative work | Hierarchy cells / builds started | Coarse / fine expansions |
| --- | ---: | --- | --- |
| Open | 5,000 | 0 / 0 | 0 / 0 |
| Obstacle dense | 19,628,032 | 8,896 / 153 | 1,427 / 27,273 |
| Bottleneck | 19,628,032 | 26,361 / 419 | 21,278 / 60,326 |
| Simultaneous replanning | 18,649,992 | 54,718 / 873 | 22,981 / 60,882 |

`NavigationWork` now reports actual chunk-build starts, cells visited while
building connectivity, and non-stale coarse/fine node expansions. Coarse counts
include the reverse connectivity probe. These counters are distinct from
cooperative future polls and from total CPU time; the 32,768-unit allowance is
not a wall-time cap. Snapshot copying, scheduler sorting, cache invalidation,
and other bookkeeping are included in tick timing but not equivalent to graph
expansions.

## Measured tuning and route comparison

The pilot exposed duplicated partial chunk construction and repeated scheduler
scans/fragmented progress across thousands of long searches. Partial builds
are now shared by requests using the same physical snapshot and discarded on
geometry refresh. Clearing-specific snapshots retain separate caches. The
scheduler sorts once per advance, using submission age plus urgency and stable
request-ID ties, and spends remaining allowance on the selected request.
New urgent work can preempt an unfinished routine search; aging eventually
lets routine work outrank new urgent arrivals. Cell width 72, chunk width 8,
and the 32,768-unit budget remain the measured configuration.

The comparable old-formation 600-tick runs are retained in
`scale/shared-cache-only/` and `scale/old-formation/`. With shared construction
held constant, the scheduler change reduced bottleneck p95 from 30.683 to
10.425 ms and completed 23 searches instead of zero. Replanning p95 fell from
26.818 to 10.628 ms with 46 replacement completions instead of zero. Dense-map
p95 fell from 23.677 to 10.240 ms, but completed searches fell from 1,200 to 14:
finishing older expensive work delays cheaper later requests. This is an explicit
throughput/latency tradeoff, not universal improvement. The final near-obstacle
formation above is a different workload and is not used for those before/after
claims. The original 60-tick pilot remains in `scale/pilot/`.

The verification-only flat A* shares physical clearance predicates but does
not use production connectivity, chunks, or search. It uses the same eight
fine-grid neighbors, distance costs, owned Corridor multiplier 0.35, and valid
straight-line alternative. Endpoints are literal fine-cell centers away from
Corridor boundaries. Independent literal checks cover 144 ordinary cost, 50.4
painted Hauler cost, a 347.64676 obstacle detour, and a 415.8 Hauler cost
across a fine-cell paint boundary. The reference is not a
second production routing mode.

Representative routes run from cell (-90,-20) to (120,20) on the same obstacle
layouts, with an owned Corridor along intent row zero. Each hierarchical route
is checked for clear segments and arrival, then repeated with warmed connectivity.

| Layout / cost type | Flat expansions | Coarse + fine expansions | Hierarchical / flat cost | Warm hierarchy / flat time (ms) |
| --- | ---: | ---: | ---: | --- |
| Open / ordinary | 0 | 0 | 1.00000 | 0.00041 / 0.00009 |
| Open / Hauler | 9,764 | 2,323 | 1.00119 | 5.309 / 2.945 |
| Dense / ordinary | 7,572 | 1,738 | 1.03834 | 74.043 / 80.512 |
| Dense / Hauler | 8,662 | 2,077 | 1.00119 | 84.968 / 93.207 |
| Bottleneck / ordinary | 7,346 | 1,898 | 1.00000 | 5.520 / 4.617 |
| Bottleneck / Hauler | 9,722 | 2,316 | 1.00119 | 8.405 / 6.637 |
| Replanning / ordinary | 20,877 | 2,287 | 1.00444 | 10.301 / 9.659 |
| Replanning / Hauler | 67,625 | 5,125 | 1.00085 | 34.872 / 33.260 |

These long routes demonstrate reduced search expansions, with measured detours
up to 3.834% in the sampled cases. They do not demonstrate universally lower
wall time. Cold hierarchy maintenance can exceed flat search work: the replanning
Hauler route visits 131,166 hierarchy cells across 2,073 build starts in addition
to 5,125 search expansions, taking 110.394 ms. Cold/warm counters and timings
for all cases are retained in `report.json`; warm routes rebuild zero cells.

## Regression and final verification

Four added tests in `tests/behavior/navigation_budget.rs` exercise the approved
public Navigation seam: cold versus warm accounting, concurrent finite service
with shared connectivity, urgent preemption plus aged detour completion, and
geometry edits while connectivity is partially built. These are scheduler and
cache regressions; gameplay flow is exercised by the named existing scripted
playtests in the matrix and by fresh real-process acceptance.

The initial red concurrency experiment observed zero of 32 completed detours under
its provisional 100,000-unit allowance. The final regression uses a finite drain
and an independently bounded physical cell count instead of that provisional
throughput threshold. All six navigation-budget tests pass, including the two
existing demand/placement cases.

Both independent reviewers found a cost-model mismatch in the flat reference's
straight-line alternative: it sampled 512-unit paint boundaries instead of
72-unit navigation-cell centers. A new literal crossing assertion failed with
410.59998 versus expected 415.8 (`target/issue-66/oracle-red.log`), then passed
with the corrected reference. The complete 600-tick benchmark and all route
comparisons were regenerated in `scale-final/`; this is the evidence used above.
Reference polling also has a deterministic 10,000-advance limit with diagnostics.

Passed commands:

```text
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test screenshots -- --ignored
cargo build --bin top-down-2d-rts-prototype-nano-swarm --example local_avoidance
python3 scripts/navigation_runtime_acceptance.py --output target/issue-66/runtime/final
python3 -m py_compile scripts/navigation_runtime_acceptance.py
git diff --check
```

`cargo test` passed 363 unit, 518 behavior, and 49 scripted playtests. Two GPU
playtests and 37 screenshot trials are ignored in the default run; the separate
offscreen run passed all 37. The primary agent re-opened all 22 relevant captures
from the final production build and confirmed the visual facts recorded above.
Clippy was rerun after the verification-only reference correction; the production
and simulation tests were unchanged by that correction. Logs are under
`target/issue-66/`.

The final real-process replay, its exact commands/state, and all seven inspected
captures are documented in [issue-66-runtime.md](issue-66-runtime.md). It shows
ordinary economy/construction/production and deterministic opposing traffic at
retreat tick 23 and arrival tick 244. Both final processes exited zero, removed
their sockets, released advisory locks, and cleaned private runtime directories.

## Standards

Independent re-review: zero unresolved findings. The corrected cost oracle and
bounded reference polling resolve the reported correctness issue. A non-blocking
design observation remains: chunk builds create full Navigation snapshots and
clone obstacle vectors; a smaller shared geometry snapshot could reduce allocation.

## Spec

Independent re-review: zero unresolved findings. The corrected equivalent-cost
baseline and regenerated evidence satisfy the comparison requirement. The reviewer
also independently inspected cancellation phases and traffic yield/arrival images.

Final review: Standards 0 unresolved; Spec 0 unresolved. The measured backlog,
cold-maintenance cost, and scheduler throughput tradeoff remain documented limits.
