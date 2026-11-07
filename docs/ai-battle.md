# AI Battle

Select **AI Battle** in the scenario menu to watch two automatically controlled swarms. Each swarm starts with the same Strategic Controller and owns its own intent layer, so painting and erasing are disabled. Camera controls, state inspection, screenshots, menu controls, and clean shutdown remain available through the [agent control interface](agents/agent-control.md).

## Run a battle

```bash
cargo run --release -- --headless --agent-socket --scenario ai-battle --seed 42 --output-root target/battle-runs
```

AI Battle is an ordinary symmetric self-play scenario. Both swarms use the same live [balance configuration](../config/balance.ron), shared with Standard and Sandbox. `--tuning-file PATH` selects a separate configuration for an experiment; accepted live edits save to that file.

Headless runs use normal pacing by default. Add `--unbounded` to advance one 60 Hz simulation tick per application update without frame sleeping; this startup option applies to Standard and Sandbox as well:

```bash
cargo run --release -- --headless --unbounded --scenario ai-battle --seed 42
```

Choose one of the two balanced layouts with `--layout standard` or `--layout flanks`:

```bash
cargo run --release -- --headless --scenario ai-battle --layout flanks --seed 11 --output-root target/battle-runs
```

`--layout` requires `--scenario ai-battle`; `--unbounded` requires `--headless`. A battle ends when Swarm Elimination resolves it. There is no automatic time cutoff; Ctrl+C records an interrupted result without inventing an outcome.

## Recorded results

Each AI Battle writes `samples.csv` and `summary.json` below the output root. Samples use simulated time and include population, births, deaths, resources, structures, damage, controller telemetry, and timing. A completed elimination, an interruption, and an in-progress recording remain distinct states.

`summary.json` uses schema version 6. `tuning` contains the full native-unit configuration at battle start. `tuning_edits` records accepted changes with `revision`, `key`, `old_value`, `value`, and `fixed_tick`. Edit values use the editor's display units; ticks are relative to the current battle's start. Earlier-session edits are excluded, and a fresh battle snapshots the current configuration. Unchanged assignments and rejected edits add no revision.

Compare the initial configuration and edit history as well as the seed when comparing runs. The seed improves repeatability but does not guarantee identical outcomes. Inspect economy, sustained attacks, recovery, and finishing behavior when evaluating a run.
