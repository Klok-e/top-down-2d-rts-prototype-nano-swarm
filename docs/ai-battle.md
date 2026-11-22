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

`--layout` and `--max-ticks` require `--scenario ai-battle`. The `--unbounded` option requires `--headless`. A battle ends when Swarm Elimination resolves it. Use `--max-ticks N` to stop after N fixed ticks. The recorder distinguishes a tick limit from elimination and from a process interruption.

For fast gameplay diagnostics, run AI Battle without graphics or frame pacing. For example, this runs Standard for up to 7,200 fixed ticks (120 simulated seconds):

```bash
cargo run --release -- --simulation-only --scenario ai-battle --layout standard --seed 11 --max-ticks 7200
```

For verification, apply the [test duration and interactive socket guidance](agents/testing.md#commands). Use rendered self-play to verify outcomes in the player runtime. Runs with the same seed can follow different trajectories across modes.

## Recorded results

Each AI Battle writes `samples.csv` and `summary.json` below the output root. Samples use simulated time and include population, births, deaths, resources, structures, damage, controller telemetry, and timing. A completed elimination, an interruption, and an in-progress recording remain distinct states.

The economy columns are recorded separately for each swarm. Window totals cover the simulated ticks since the previous sample, including a partial window at the end of a run.

| Column | Measurement |
| --- | --- |
| `owned_deposits` | Distinct Resource Deposits that yielded minerals to the swarm while covered by its Gather paint during the window. |
| `deposit_extraction` | JSON array of minerals actually extracted by the swarm from each deposit since battle start, including zeroes. Each entry contains the grid `cell`, deposit `owner` (`null` for neutral deposits), and `minerals_extracted`. |
| `mineral_income` | Minerals the swarm extracted during the window. |
| `mineral_starved_facility_seconds` | Simulated seconds summed across operational Production Facilities that had unmet Population Demand and too few minerals in their input buffer to start production. Two such facilities idle for one second contribute two facility-seconds. |
| `unspent_minerals` | Minerals held by the swarm at sample time in surviving Stockpiles, Production Facilities, Chargers, and Nanobot cargo. |
| `empty_charge_defenders` | Defenders at zero Charge at sample time. |

`summary.json` uses schema version 8. `tuning` contains the full native-unit configuration at battle start. `tuning_edits` records accepted changes with `revision`, `key`, `old_value`, `value`, and `fixed_tick`. Edit values use the editor's display units; ticks are relative to the current battle's start. Earlier-session edits are excluded, and a fresh battle snapshots the current configuration. Unchanged assignments and rejected edits add no revision.

Compare the initial configuration and edit history as well as the seed when comparing runs. The seed improves repeatability but does not guarantee identical outcomes. Inspect economy, sustained attacks, recovery, and finishing behavior when evaluating a run.
