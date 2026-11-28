# Navigation verification

## Find a regression test

| Symptom | Where to check |
| --- | --- |
| A bot clips a wall or deposit, loses a valid route after an unrelated edit, or ignores an owned Corridor on its next trip | [Shared navigation playtests](../../tests/playtest/shared_navigation.rs) |
| Route requests starve, use stale geometry, or consume too much work in one update | [Budget and scheduling](../../src/navigation/budget.rs) and [integration tests](../../tests/behavior/navigation_budget.rs) |
| A Worker or Hauler enters a structure to perform work, drops cargo on a blocked route, or fails to resume after access reopens | [Exterior work flow](../../tests/playtest/exterior_work_flow.rs), [route recovery](../../tests/behavior/worker_route_recovery.rs), and [physical deliveries](../../tests/playtest/physical_logistics_flow.rs) |
| Opposing traffic wedges in a passage or a queue at a work site stops making deliveries | [Local avoidance](../../tests/behavior/local_avoidance.rs), [single-file transit](../../tests/behavior/congestion.rs), and [sustained economy playtests](../../tests/playtest/congested_economy.rs) |
| A plan cuts off friendly access or becomes solid around a bot | [Construction access](../../tests/behavior/construction_access.rs), [clearing](../../tests/behavior/structure_clearing.rs), and [cancellation](../../tests/playtest/construction_cancellation.rs) |
| A facility loses or duplicates paid output while its exit is blocked | [Production exit playtests](../../tests/playtest/production_exits.rs) |

## Replay gameplay, delivery lanes, and Structure Clearing

Apply the offscreen verifier setup. Each scene gets a fresh configuration directory under its output directory and starts in Standard. The driver creates and removes a private temporary runtime directory for each scene. To choose its parent with `--runtime-dir`, use a real directory you own with mode `0700` and a short path so the control socket path fits the Unix limit. Concurrent runs can share this parent. [Offscreen verifier setup](../agents/agent-control.md#offscreen-verifier-setup)

```bash
mkdir -p .tmp &&
  replay_dir=$(mktemp -d "$PWD/.tmp/navigation-replay.XXXXXX") &&
  cargo build --example local_avoidance &&
  RUST_LOG=local_avoidance=info python3 scripts/navigation_runtime_acceptance.py --output "$replay_dir"
```

In `$replay_dir/normal/`, compare the `start`, `working`, `delivery`, and `later` captures. Look for Workers around the deposit, plans becoming solid structures, and Haulers working outside them. Compare the player's mineral balance and facility counts across the JSON snapshots for evidence of economy progress. Phase names mark timed samples. Screenshots and snapshots can show different ticks.

For each scene below, compare the `before`, `passing`, and final captures with their detailed state snapshots under `$replay_dir/<scene>/`. Use `--scene <scene>` to run one scene. The fixtures check swept clearance. The driver checks process exit, socket removal, and lock release for every scene. Run `cargo test --example local_avoidance` to verify the independent segment-to-rectangle clearance calculation. [Replay fixtures](../../examples/local_avoidance.rs)

| Scene | Passing | Final capture |
| --- | --- | --- |
| `bottleneck` | Two loaded Haulers pass in opposite directions and the Hauler at the Source Stockpile continues loading. | `arrived` |
| `clearing` | Two occupants take the same narrow exit through a stationary friendly Nanobot. One body remains in the footprint at the pause. | `activated` |
| `direct` | A loaded eastbound Hauler bends around westbound traffic on open ground. | `arrived` |
| `searched` | A solid barrier blocks the direct route. Westbound traffic uses the lower opening while the eastbound Hauler uses the quiet upper opening. | `arrived` |

Check each `arrived` snapshot for receipts at the destination Production Facilities. At `activated`, the footprint is empty and the Source Stockpile is operational.

For manual replay, set `NANO_SWARM_REPLAY_SCENE` to the scene name, `RUST_LOG` to `local_avoidance=info`, and `NANO_SWARM_REPLAY_TUNING` to a private copy of `config/balance.ron`. Launch `cargo run --example local_avoidance` with private configuration and runtime directories. Use the matching client socket to paint Corridor at `-3 -3` to start and at `-2 -3` after the `passing` log. [Client commands](../agents/agent-control.md#client)

## Compare crowded delivery across revisions

Use `5b61292` for pre-change pathing and the final revision containing Transit Overlap with the Traffic Penalty for the combined run. Copy `examples/crowded_delivery.rs` and `config/balance.ron` from the combined checkout into the other two checkouts. For Transit Overlap alone, start from the combined revision and temporarily replace construction and population of the per-tick snapshot in `move_velocity_system` with `let traffic = Arc::new(TrafficSnapshot::default());`. Retain the route-start logic from the combined revision. Use the same machine across all three variants. Save one report per variant and include the exact commits and patch used with the results. [Benchmark source](../../examples/crowded_delivery.rs) · [Movement routing](../../src/nanobot/work_navigation.rs)

```bash
mkdir -p .tmp &&
  delivery_dir=$(mktemp -d "$PWD/.tmp/crowded-delivery.XXXXXX") &&
  cargo run --release --example crowded_delivery > "$delivery_dir/report.json"
```

Every run uses seeds 1, 11, 42, 73, and 117, a 48 × 48 map, 24 starting Haulers, one pickup stocked with 2,000 minerals, three empty Production Facilities, and two single-file lanes. Each seed advances 7,200 fixed ticks (120 simulated seconds). Deliveries count minerals in facility hoppers plus minerals already consumed in production. Births count releases into the world. Crossings count each Hauler passage by lane and direction. Timing covers the simulation update, excluding the crossing measurements.

Subtract the pre-change result from the combined result for each seed. The median paired difference must be positive for both delivered minerals and released births. For each matching seed, compare crossings by lane and direction between the combined variant and Transit Overlap alone. Report p50 and p95 fixed tick wall time for each seed and variant.

## Measure navigation at scale

```bash
mkdir -p .tmp &&
  bench_dir=$(mktemp -d "$PWD/.tmp/navigation-bench.XXXXXX") &&
  NAV_BENCH_OUTPUT="$bench_dir" cargo bench --bench navigation_acceptance
```

The benchmark runs 5,000 Workers on open terrain, among dense obstacles, through a bottleneck, and with an obstacle introduced during travel. `NAV_BENCH_TICKS` sets the observation window (default 600). Use at least 31 ticks to exercise the obstacle insertion. Keep the tick count and machine comparable across revisions. Each update checks swept obstacle clearance and the 32,768-unit navigation work limit. The measured simulation excludes the renderer, controller, allocation, and economy.

In `$bench_dir/report.json`, compare each scenario’s p95 tick time with its final queue (`final.pending`) and the number of distinct bots that reached x > 72 (`crossed_x72`). In the per-scenario JSON file, sum `ticks[].completed` for the total number of requests resolved, including unreachable results. Each tick’s maximum completion latency excludes requests still waiting. A faster tick can coincide with less traffic getting through. These workloads have no universal frame-time or completion threshold.

In `route_comparison`, compare path cost, search expansions, and elapsed time against the independent flat A* reference. Cold runs also pay to build hierarchy data. Fewer search expansions can still take longer or yield a higher-cost route. [Benchmark](../../benches/navigation_acceptance.rs) · [Flat reference](../../benches/navigation_acceptance/reference.rs)
