# AI Battle

Select **AI Battle** in the scenario menu to watch two automatically controlled swarms. Painting and erasing are disabled; camera controls remain available. Normal AI Battle uses the Timed pair on Standard with Baseline pacing. Standard player-versus-AI uses Adaptive; Standard and Sandbox use shared Deliberate pacing. Acceptance checks are defined in [ADR-0022](adr/0022-strategic-controller-objective.md) and [issue #72](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/72).

## Real-process checks

Launch an unlimited headless battle with:

```bash
cargo run --release -- --scenario ai-battle --headless --seed 42 --output-root target/battle-runs
```

Headless AI Battle uses offscreen rendering and advances as fast as possible. The first Swarm Elimination ends recording and exits the headless process; normal AI Battle has no time limit. Ctrl+C records an interrupted run without inventing a winner. A seed improves repeatability but does not guarantee identical outcomes.

For a bounded controller check:

```bash
cargo run --release -- --headless --agent-socket --scenario ai-battle --experiment --controllers adaptive,timed --layout standard --seed 11 --pacing deliberate --trial-seconds 600 --output-root target/ai-smoke
```

Experimental trials stop at the configured simulated-time cutoff and record `unresolved` with no game outcome. Natural elimination takes precedence. `--swap-sides` exchanges controllers between physical starts; `--realtime` enables normal-speed offscreen observation. Supported controller choices are Adaptive and Timed. Standard, Flanks, Narrows, and Crossroads are controlled layouts; they are not reserved evaluation inputs.

Use [the canonical control client](agents/agent-control.md) for state inspection, camera controls, captures, and shutdown. Automated verification never creates an OS/compositor window. Copy captures out of a temporary runtime directory before its cleanup and inspect the images. Accelerated screenshots alone do not establish normal-speed readability.

## Small smoke matrix

The runner defaults to four games: Adaptive against Timed, Standard/Flanks, seed 11, both physical starts, Baseline pacing, and a 600-second cutoff:

```bash
python scripts/ai_battle_experiment.py --output target/ai-smoke-matrix
python scripts/ai_battle_experiment.py --pacing deliberate --output target/ai-smoke-deliberate
```

Explicit `--controller`, `--opponent`, `--layouts`, `--seeds`, `--pacing`, and `--trial-seconds` select another bounded check. Hold controller matchups fixed when comparing shared pacing. Do not repeat smoke matrices solely to obtain a better score; inspect economy, sustained attacks, recovery, and finishing instead.

Freeze source and configuration for the whole matrix. Each release-build game has a private runtime directory, process log, and successful-exit receipt. The manifest and source bundle identify exact dirty source, settings, and trials. Ignored output directories such as `target/` prevent artifacts from changing the source fingerprint. Re-run an unchanged command to resume; do not restart a live process merely because observation timed out. Failed or interrupted runs require investigation, not reclassification as competitive outcomes.

`--report-only` validates an existing matrix against its recorded manifest and source bundle without requiring the current source to match. Reports contain actual damage, outcomes, and per-layout/start observations, not statistical promotion decisions. Historical research artifacts and source bundles remain under their original output paths; removed policy versions and old report schemas are not compatibility interfaces.

## Controller and shared pacing

Adaptive receives immutable world observations and returns only its owner's intent edits. Its stateful planner keeps one active resource site, retargets exhausted or lost deposits, coordinates economic and combat intent, and retains useful attacks. Proactive multi-site expansion is omitted to limit economic complexity. The shared simulation chooses individual work assignments, movement, resupply, and attacks. Full-state observation does not grant direct mutation authority.

Planning shares 100,000 accounted work units per 30-tick review window, including urgent checks. Exhaustion retains existing intent until a fresh allowance is available. Telemetry records review count, edits, work, explanation, mean/max cost, and p95 over at most 4,096 recent reviews; `p95_window_samples` gives the actual count. Observation cost is measured separately.

`baseline` retains the original shared timing values; `deliberate` uses 90 construction ticks, 30 ticks between attacks, and 0.000125 Charge drain per tick. Both apply to every swarm. Summaries record numeric values, not just profile names. A profile's existence is not evidence of good feel; inspect normal-speed response, travel, construction, resupply, and combat before selecting it.

## Recorded evidence

Each battle writes `samples.csv` and `summary.json`. Sampling uses simulated seconds, includes the final partial interval, and freezes at termination or experiment cutoff. Metadata identifies scenario, seed, physical swarm identities, controllers, settings, revision, and dirty state. Wins, losses, Draws, unresolved attempts, and technical interruptions remain distinct.

Population counts are current; births, deaths, gathered/consumed minerals, and built/lost structures are cumulative. Starting entities are excluded from birth/construction totals. Cargo loss is not resource consumption. Planned structures are not completed structures.

Damage schema 3 records gross `effective_damage_total` and lifetime-capped `scored_damage_total`, each with Nanobot and structure subtotals. Only actual hostile combat HP removed counts: no overkill, friendly/self damage, Charge attrition, maintenance decay, or unfinished-structure damage. All attackers share one maximum-health-bar credit limit per enemy lifetime; repair cannot renew it. Old records without authoritative counters cannot be interpreted as zero damage.

Damage is diagnostic, not a shipping contest. Gross-versus-scored differences expose repeat damage; they do not prove intentional farming. A per-entity cap cannot prevent farming newly produced enemies, so inspect viable finishing opportunities and retain autonomous finishing tests. Unresolved games are not wins or Draws, and an early loss is not fast completion.

Tick timings measure fixed simulation through outcome evaluation, excluding report serialization and writes. Normal-speed frame timings include pacing; accelerated frame fields are empty. CSV history retains transient maxima. Report measured runtime cost, but do not apply the superseded statistical promotion or 128-game evaluation gates. Current acceptance and historical observations are separated in the [verification record](verification/strategic-controller.md).
