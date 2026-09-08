# AI Battle

Select **AI Battle** in the scenario menu, then relaunch to watch two automatically controlled swarms on the Standard map. Painting and erasing are disabled; camera controls remain available. Both sides use the same timed intent controller and shared economy, navigation, and combat rules.

Launch a headless benchmark with:

```bash
cargo run --release -- --scenario ai-battle --headless --seed 42 --output-root target/battle-runs
```

Omit `--headless` for normal-speed watching. Both modes use 60 simulation ticks per simulated second and the supplied starting seed (default `0`). Headless AI Battle advances as fast as possible; it still uses the game's offscreen GPU renderer. Compare simulation timings between like configurations and builds. A fixed seed improves repeatability but does not guarantee identical battle outcomes.

There is no time limit. The first Swarm Elimination records the winning swarm, or Draw for simultaneous elimination, and ends statistics collection. Watchable simulation continues; headless saves and exits. Ctrl+C saves an unfinished run as `interrupted`; it does not declare a winner or Draw. A stalemate can run indefinitely until interrupted.

Each run creates a unique directory under the output root containing `samples.csv` and `summary.json`. CSV rows are sampled once per simulated second, with one row per swarm, and a final sample on termination even between regular sample boundaries. Summary metadata records scenario, seed, timestep, execution mode, code revision, and whether the source checkout contains changes. Periodic flushes preserve completed samples; graceful interruption flushes the current partial interval.

Population columns are current counts by Nanobot Type. Births, deaths, minerals gathered/consumed, and structures built/lost are cumulative since the start of the run. Starting Nanobots and structures are excluded from birth/construction counters. Cargo lost when its carrier dies is not resource consumption. Planned Structures are excluded from completed structure counts. Elimination time is absent for a swarm that has not been eliminated when recording ends.

Tick timing is measured in wall-clock milliseconds over each sampling interval, with count, mean, nearest-rank p50/p95/p99, and maximum. It measures fixed simulation work through outcome evaluation, excluding report serialization and disk writes. It is not a per-system profiler. Watchable frame timing measures intervals between application frames, including pacing; headless frame fields are empty. Summary contains the latest sample; the CSV retains the full history. Post-outcome simulation never appends additional samples.

For background automated observation, add `--agent-socket` and use the existing agent-control client. Automated verification must use offscreen presentation without an OS window. See [agent control](agents/agent-control.md) for socket setup and capture commands, and [ADR-0021](adr/0021-ai-battle-benchmark.md) for the agreed scope.
