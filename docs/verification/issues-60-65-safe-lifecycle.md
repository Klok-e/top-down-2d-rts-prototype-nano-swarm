# Budgeted navigation and safe structure lifecycles

Verified 2026-09-05 against issues [#60](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/60), [#62](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/62), [#63](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/63), [#64](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/64), and [#65](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/65). The accepted policies are in ADR-0016 and ADR-0017. This completes the approved dependency chain for the two remaining #59 gameplay failures. Cargo recovery (#61) and the 5,000-nanobot scale acceptance (#66) are outside this continuation.

## Behavior and regression evidence

- Navigation schedules resumable requests with explicit Pending, Found, and Unreachable outcomes, urgency and aging, and affected-region invalidation. Geometry predicates, hierarchy maintenance, route costs, and composite construction-access checks consume the shared work budget. Zero-budget integration checks preserve resources and demand without prematurely assigning unreachable outcomes or approving construction; reopening the budget permits progress.
- Measured deterministic fixtures: one detour takes 146 ticks at 100 work units per tick; 24 simultaneous replans complete in 269 ticks at a 1,000-unit cap, with maximum reported request latency 270 ticks. These are work-accounting checks, not wall-time or 5,000-agent benchmarks.
- Local movement enforces swept static and 68-unit pair clearance. Deterministic yielding, backout, and return to retained routes resolve opposing passage traffic without crowd-based global detours. Seven tests cover open space, narrow and sealed passages, parallel lanes, four-bot streams under three spawn orders, stopped navigation budgets, and moving pursuit targets.
- Construction preserves previously connected useful friendly endpoints with all plans completed hypothetically. It checks builder access, retries alternatives, and invalidates cached decisions on relevant layout changes. Ordinary plans remain traversable while access is pending.
- Finished work releases its Worker immediately. Validated Clearing bars entrants, evacuates either swarm, waits without a congestion timeout, and revalidates before activation. Rejection releases the reservation and displays a red pulse followed by a shrinking, fading outline. Same-pass cancellations retain every rejected-site exclusion until an external layout change; a regression failed before the normalization fix and passed afterward.
- Production retains one paid, typed output while exits are blocked, pauses further cycles, counts that commitment without making it available for work, and does not treat exit waiting as busy expansion capacity. Tests exercise actual mineral payment, destruction without output/refund, clearing-blocked exits, and oldest-output priority at a contested exit.

Both `authored_charger_planning_and_maintenance_follow_observed_service_need` and `authored_default_scenario_reaches_primary_defend_contest` pass. Their gameplay assertions remain. Tests now wait within explicit bounds for queued route/access readiness, and service fixtures use exterior positions. Source tests retain exact placement, jitter, mineral-conservation, and duplicate-commitment assertions. Combat fixtures preserve hit and contact outcomes while separating initially overlapping bodies.

The authored start now uses 72-unit-spaced 3×3 formations for each swarm instead of nine coincident bodies. Counts, types, intent, and structures remain the same. Literal position and spacing assertions cover this change.

## Final gates

Commands used `CARGO_INCREMENTAL=0`; no OS/compositor window was created.

| Check | Result |
| --- | --- |
| `cargo fmt`, final formatting check | Passed |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo test` | 363 unit + 492 behavior + 38 playtests passed; 2 GPU playtests and 36 screenshot trials ignored by default |
| `cargo test --test playtest -- --ignored` | Both GPU playtests passed |
| `cargo test --test screenshots -- --ignored` | All 36 offscreen trials passed |
| Standards and Spec review | No remaining confirmed defects after fixes and independent re-review |

Review found three defects that were fixed with regression coverage: Clearing was omitted from navigation/production query filters; simultaneous cancellation could erase another site's exclusion; hypothetical construction routing bypassed the navigation budget. Follow-up review checked the corrected integration. `docs/agents/testing.md` was not changed.

## Inspected rendering and real runtime

Fresh focused captures under `target/playtest-screenshots/` show:

- `construction_access_{open_passage,safe_alternative}.png`: preserved wall passage and an alternative Charger outline outside it.
- `construction_clearing_{occupied,activated}.png`: an occupant leaves before the completed structure appears.
- `construction_cancellation_{pulse,collapse,fade}.png`: red cancellation outline shrinks and disappears without retaining a structure.
- `production_exit_{waiting,released}.png`: surrounded facility retains output; a Worker vacates an exit and one Hauler appears there.
- `local_avoidance_{before,yield,arrived}.png`: opposing traffic backs out through the right mouth, passes above the wall, and finishes on opposite sides with separated bodies.
- `startup_formations.png`: distinct starting bodies beside each facility.

Focused images were inspected individually; contact sheets also covered the screenshot directory, which contains both current and historical artifacts. File count is not used as a fresh-render count.

A real `cargo run -- --headless --agent-socket --width 1280 --height 720` process used a private mode-0700 `/tmp/ns-proof.*` runtime directory and `scripts/nano_swarm_control.py`. At tick 6 both swarms were in progress with four Workers, two Haulers, three Defenders, and zero minerals each. After painting Defend at (1, 1) and waiting 300 fixed ticks, tick 322 showed both still in progress, unchanged populations, and player/opponent minerals 136/48. Inspected before/after captures show separated startup bodies, exterior gathering/transport, and Defender movement toward the paint. Shutdown at tick 330 exited with code 0; socket removal, released lock, and runtime-directory cleanup were verified.

An earlier unsteered run was already in Defeat at tick 4,966. It is retained as `chain_long_run_defeat.png`; the passing authored acceptance horizon and short input proof do not establish indefinite economy stability or guaranteed victory.

Uncommitted evidence is collected in `target/chain-verification/`: final Cargo logs, budget measurements, `runtime-proof.log`, `runtime-commands.jsonl`, runtime screenshots, and contact sheets. The real-runtime driver is `verify_runtime.py` in that directory.
