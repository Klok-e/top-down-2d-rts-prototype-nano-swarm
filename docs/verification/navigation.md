# Navigation verification

## Find a regression test

| Symptom | Where to check |
| --- | --- |
| A bot clips a wall or deposit, loses a valid route after an unrelated edit, or ignores an owned Corridor on its next trip | [Shared navigation playtests](../../tests/playtest/shared_navigation.rs) |
| Route requests starve, use stale geometry, or consume too much work in one update | [Budget and scheduling](../../src/navigation/budget.rs) and [integration tests](../../tests/behavior/navigation_budget.rs) |
| A Worker or Hauler enters a structure to perform work, drops cargo on a blocked route, or fails to resume after access reopens | [Exterior work flow](../../tests/playtest/exterior_work_flow.rs), [route recovery](../../tests/behavior/worker_route_recovery.rs), and [physical deliveries](../../tests/playtest/physical_logistics_flow.rs) |
| Opposing traffic wedges in a passage or a queue at a work site stops making deliveries | [Local avoidance](../../tests/behavior/local_avoidance.rs), [congestion recovery](../../tests/behavior/congestion.rs), and [sustained economy playtests](../../tests/playtest/congested_economy.rs) |
| A plan cuts off friendly access or becomes solid around a bot | [Construction access](../../tests/behavior/construction_access.rs), [clearing](../../tests/behavior/structure_clearing.rs), and [cancellation](../../tests/playtest/construction_cancellation.rs) |
| A facility loses or duplicates paid output while its exit is blocked | [Production exit playtests](../../tests/playtest/production_exits.rs) |

## Replay gameplay and bottleneck traffic

From the repository root, apply the [offscreen verifier setup](../agents/agent-control.md#offscreen-verifier-setup). The replay requires an offscreen GPU adapter. A fresh configuration directory selects Standard regardless of the player's saved scenario.

```bash
mkdir -p .tmp &&
  replay_dir=$(mktemp -d "$PWD/.tmp/navigation-replay.XXXXXX") &&
  cargo build --bin top-down-2d-rts-prototype-nano-swarm --example local_avoidance &&
  RUST_LOG=local_avoidance=info XDG_CONFIG_HOME="$replay_dir/config" python3 scripts/navigation_runtime_acceptance.py --output "$replay_dir"
```

In `$replay_dir/normal/`, compare the `start`, `working`, `delivery`, and `later` captures. Look for Workers around the deposit, plans becoming solid structures, and Haulers working outside them. Compare the player's mineral balance and facility counts across the JSON snapshots for evidence of economy progress. Phase names mark timed samples. Screenshots and snapshots can show different ticks.

Under `$replay_dir/bottleneck/`, compare `before.png`, `retreat.png`, and `arrived.png`. Look for a bot yielding at the mouth of the passage and all four reaching their opposite-side goal markers. The [fixture](../../examples/local_avoidance.rs) checks swept structure clearance on every tick and body separation outside friendly congestion recovery. It documents the two paint commands for advancing the scene manually. The driver checks process exit, socket removal, and lock release for both scenes. To test the fixture’s independent segment-to-rectangle distance calculation, run `cargo test --example local_avoidance`.

## Measure navigation at scale

```bash
mkdir -p .tmp &&
  bench_dir=$(mktemp -d "$PWD/.tmp/navigation-bench.XXXXXX") &&
  NAV_BENCH_OUTPUT="$bench_dir" cargo bench --bench navigation_acceptance
```

The benchmark runs 5,000 Workers on open terrain, among dense obstacles, through a bottleneck, and with an obstacle introduced during travel. `NAV_BENCH_TICKS` sets the observation window (default 600). Use at least 31 ticks to exercise the obstacle insertion. Keep the tick count and machine comparable across revisions. Each update checks swept obstacle clearance and the 32,768-unit navigation work limit. The measured simulation excludes the renderer, controller, allocation, and economy.

In `$bench_dir/report.json`, compare each scenario’s p95 tick time with its final queue (`final.pending`) and the number of distinct bots that reached x > 72 (`crossed_x72`). In the per-scenario JSON file, sum `ticks[].completed` for the total number of requests resolved, including unreachable results. Each tick’s maximum completion latency excludes requests still waiting. A faster tick can coincide with less traffic getting through. These workloads have no universal frame-time or completion threshold.

In `route_comparison`, compare path cost, search expansions, and elapsed time against the independent flat A* reference. Cold runs also pay to build hierarchy data. Fewer search expansions can still take longer or yield a higher-cost route. [Benchmark](../../benches/navigation_acceptance.rs) · [Flat reference](../../benches/navigation_acceptance/reference.rs)
