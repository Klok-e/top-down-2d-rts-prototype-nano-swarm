# AI Battle

Select **AI Battle** in the scenario menu to watch two automatically controlled swarms. Each swarm starts with the same Strategic Controller and owns its own intent layer, so painting and erasing are disabled. Camera controls, state inspection, screenshots, menu controls, and clean shutdown remain available through the [agent control interface](agents/agent-control.md).

## Run a battle

```bash
cargo run --release -- --headless --scenario ai-battle --seed 42
```

Every launch opens the agent socket. No AI Battle writes files or exits by itself. Read results over the socket with [`battle.stats`](agents/agent-control.md#battle-statistics) and stop the process with `shutdown`.

AI Battle is an ordinary symmetric self-play scenario. Both swarms use the same live [balance configuration](../config/balance.ron), shared with Standard and Sandbox. `--tuning-file PATH` selects a separate configuration for an experiment; accepted live edits save to that file.

Headless runs use normal pacing by default. Add `--unbounded` to advance one 60 Hz simulation tick per application update without frame sleeping; this startup option applies to Standard and Sandbox as well:

```bash
cargo run --release -- --headless --unbounded --scenario ai-battle --seed 42
```

Choose one of the two balanced layouts with `--layout standard` or `--layout flanks`:

```bash
cargo run --release -- --headless --scenario ai-battle --layout flanks --seed 11
```

`--layout` and `--max-ticks` require `--scenario ai-battle`. The `--unbounded` option requires `--headless`. A battle ends when Swarm Elimination resolves it. Use `--max-ticks N` to end it after N fixed ticks; without it, only Swarm Elimination ends a battle.

At the end, a headless or simulation-only battle freezes its simulation. Statistics stop changing, and the process keeps serving the socket until `shutdown`, with no idle timeout. The frozen loop of any headless or simulation-only battle, including `--unbounded`, runs at about 60 iterations per second, so it does not occupy a core. A windowed battle keeps its world running with statistics frozen, and stays open until the player quits or `shutdown` arrives. The statistics distinguish a tick limit from elimination.

For fast gameplay diagnostics, run AI Battle without graphics or frame pacing. For example, this runs Standard for up to 7,200 fixed ticks (120 simulated seconds):

```bash
cargo run --release -- --simulation-only --scenario ai-battle --layout standard --seed 11 --max-ticks 7200
```

For verification, apply the [test duration and interactive socket guidance](agents/testing.md#commands). Use rendered self-play to verify outcomes in the player runtime. Runs with the same seed can follow different trajectories across modes.

## Recorded results

Each AI Battle keeps its samples, summary, and hitch summaries in memory and serves them through [`battle.stats`](agents/agent-control.md#battle-statistics); see [ADR-0031](adr/0031-in-memory-battle-statistics.md). A crash or killed process loses them. Samples use simulated time and include population, births, deaths, resources, structures, damage, controller telemetry, and timing. A completed elimination, a tick limit, and an in-progress recording remain distinct states.

`battle.stats` lists one compact summary for each [hitch episode](performance/hitch-recorder.md) that closes while the run is in progress. Its `start_frame` and `end_frame` include the pre-roll and recovery frames, and `start_fixed_tick` is the number of fixed ticks completed before the first frame in that range. `hitch_count`, `worst_frame_ns`, and `catch_up_ticks` describe the episode's hitch frames. Catch-up counts each fixed tick after the first in a hitch frame. `top_phases` ranks three phases by total nanoseconds across all hitch frames, including those omitted from retained detail. Fixed phases rank separately, and `RunFixedMainLoop` contributes its remaining time. `nanobots` is the Nanobot count at the end of the earliest worst frame.

Fetch per-frame detail from the agent socket with [`perf.hitches`](agents/agent-control.md#hitch-episodes).

Frame timing uses the hitch recorder's frame durations while the battle is in progress and virtual time is running. It is omitted in headless unbounded runs. Tick timing continues to measure each fixed tick.

The economy fields are recorded separately for each swarm. Window totals cover the simulated ticks since the previous sample, including a partial window at the end of a run.

| Field | Measurement |
| --- | --- |
| `owned_deposits` | Distinct Resource Deposits that yielded minerals to the swarm while covered by its Gather paint during the window. |
| `deposit_extraction` | JSON array of minerals actually extracted by the swarm from each deposit since battle start, including zeroes. Each entry contains the grid `cell`, deposit `owner` (`null` for neutral deposits), and `minerals_extracted`. |
| `mineral_income` | Minerals the swarm extracted during the window. |
| `mineral_starved_facility_seconds` | Simulated seconds summed across operational Production Facilities that had unmet Population Demand and too few minerals in their input buffer to start production. Two such facilities idle for one second contribute two facility-seconds. |
| `unspent_minerals` | Minerals held by the swarm at sample time in surviving Stockpiles, Production Facilities, Chargers, and Nanobot cargo. |
| `empty_charge_defenders` | Defenders at zero Charge at sample time. |
| `spending_capacity` | Minerals per second the swarm would spend at sample time: Production Facilities that are producing or have unmet Population Demand, running continuously, plus charge upkeep for its Defenders. |
| `reserve_target` | Reserve Target at sample time: `reserve_seconds` of Spending Capacity, in minerals. |
| `reserve_stock` | Minerals in the swarm's operational Source and Sink Stockpiles at sample time, each counted by how soon it could reach the nearest Production Facility or Charger (in full within `reserve_seconds`). |
| `gather_quota` | Gather Quota at sample time: the number of Workers required for gathering. |
| `gathering_workers` | Workers travelling to, working at, or returning from a deposit, or holding gathered cargo, at sample time. Includes Spare Gathering. |
| `gather_rate` | Minerals per second one productive gatherer brought in over the trailing 20 s, or `gather_rate_fallback` when that window measured nothing. |
| `haul_target_flow` | Minerals per second the last 20 s Haul Quota window found to deliver: income plus stock above `reserve_target` spread over `reserve_seconds`, at most `spending_capacity`. |
| `delivered_rate` | Minerals per second the swarm's Production Facilities and Chargers consumed over that window. |
| `haul_rate` | Minerals consumed per busy-Hauler-second over that window. |
| `haul_quota` | Haulers the swarm wants for `haul_target_flow` at `haul_rate`, limited by the growth guard; 0 before the first window. |
| `haul_growing` | Whether the growth guard lets Hauler demand exceed the current Hauler count: false for `haul_growth_retry_ticks` after added Haulers failed to raise delivery. |

The `battle.stats` summary uses schema version 11. `tuning` contains the full native-unit configuration at battle start. `tuning_edits` records accepted changes with `revision`, `key`, `old_value`, `value`, and `fixed_tick`. Edit values use the editor's display units; ticks are relative to the current battle's start. Earlier-session edits are excluded, and a fresh battle snapshots the current configuration. Unchanged assignments and rejected edits add no revision.

Compare the initial configuration and edit history as well as the seed when comparing runs. The seed improves repeatability but does not guarantee identical outcomes. Inspect economy, sustained attacks, recovery, and finishing behavior when evaluating a run.
