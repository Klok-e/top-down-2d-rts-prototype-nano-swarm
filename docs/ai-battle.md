# AI Battle

Select **AI Battle** in the scenario menu to watch two automatically controlled swarms. Each swarm starts with the same Strategic Controller and owns its own intent layer, so painting and erasing are disabled. Camera controls, state inspection, screenshots, menu controls, and clean shutdown remain available through the [agent control interface](agents/agent-control.md).

## Run a battle

```bash
cargo run --release -- --headless --agent-socket --scenario ai-battle --seed 42 --output-root target/battle-runs
```

AI Battle is an ordinary symmetric self-play scenario. It uses the shared gameplay pacing: 90 construction work ticks, a 30-tick Defender attack interval, and 0.000125 Charge drain per tick. Standard and Sandbox use the same values.

Headless AI Battle advances one 60 Hz simulation tick per application update. Add `--realtime` to run it at normal headless pacing:

```bash
cargo run --release -- --headless --scenario ai-battle --realtime --seed 42
```

Choose one of the two balanced layouts with `--layout standard` or `--layout flanks`:

```bash
cargo run --release -- --headless --scenario ai-battle --layout flanks --seed 11 --output-root target/battle-runs
```

`--layout` and `--realtime` require `--scenario ai-battle`. A battle ends when Swarm Elimination resolves it. There is no automatic time cutoff; Ctrl+C records an interrupted result without inventing an outcome.

## Recorded results

Each AI Battle writes `samples.csv` and `summary.json` below the output root. Samples use simulated time and include population, births, deaths, resources, structures, damage, controller telemetry, and timing. A completed elimination, an interruption, and an in-progress recording remain distinct states.

The seed improves repeatability but does not guarantee identical outcomes. Inspect economy, sustained attacks, recovery, and finishing behavior when evaluating a run.
