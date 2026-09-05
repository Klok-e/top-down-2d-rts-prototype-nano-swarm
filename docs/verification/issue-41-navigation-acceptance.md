# Shared navigation aggregate acceptance

Verified 2026-09-05 for [issue #41](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/41), starting at `128a1e6b7d19d7dc4d725b5d31de6f349b65cb09`.
The issue was the only open task, with no native blockers or assignee. Its implementation had landed through #57–#66. Two independent read-only audits checked navigation/crowds/performance and work/construction/production against the parent specification; neither found an outstanding behavioral requirement. This follow-up corrects the geometry document's stale claim that the consumers are not installed and records fresh aggregate acceptance. No production code or tests changed.

## Acceptance map

| Contract | Implementation and deterministic evidence | Detailed record |
| --- | --- | --- |
| Grid-aligned structures and exterior gathering, building, maintenance, pickup, delivery, charging | `InteractionRegion`; `tests/behavior/{exterior_work,exterior_haul}.rs`; `tests/playtest/exterior_work_flow.rs` | [Geometry](../navigation-geometry.md), [#57](issue-57-exterior-work.md) |
| Shared hierarchy for all types/swarms, physical structures/deposits, swept clearance, Corridor preference and stable valid routes | `Navigation`, `PhysicalWorld`, movement; `tests/playtest/shared_navigation.rs` | [#59](issue-59-shared-navigation.md), [#60](issue-60-navigation-acceptance.md) |
| Shared bounded work, pending versus unreachable, urgent service and aging, stale-result rejection, pending demand/placement | Public Navigation tests; `tests/behavior/navigation_budget.rs` | [#60](issue-60-navigation-acceptance.md), [#66](issue-66-navigation-scale.md) |
| Unavailable work, alternate destinations, physical cargo return/waiting, reservation cleanup and reopening | `work_access`; task-reachability and Worker-recovery behavior tests; physical-logistics playtests | [#61](issue-61-route-recovery.md) |
| Local yielding/backout with retained routes and safe constrained waiting | Swept movement/separation; local-avoidance behavior/playtests and runtime fixture | [#62](issue-62-local-avoidance.md) |
| Joint-plan access preservation, builder reachability, existing disconnections, friendly-only protection | Shared `AccessLayout::check`; construction-access tests through actual planners | [#63](issue-63-construction-access.md) |
| Traversable plans, clearing either swarm without teleporting, barred entrants, final-check cancellation and immediate release, exclusion/retry, visual pulse/collapse/fade | Structure lifecycle; clearing behavior, adjacent-clearing and cancellation playtests | [#64](issue-64-structure-clearing.md) |
| Finished output waits for exterior space and emits once without another cost | Production-exit behavior/playtests for both swarms and real movement opening exits | [#65](issue-65-production-exits.md) |
| 5,000-Nanobot open/dense/bottleneck/replanning measurements and independent flat-search comparison | Navigation scale benchmark and retained measurements | [#66](issue-66-navigation-scale.md) |

The existing `shared_navigation::ranged_pursuit_uses_an_accessible_side_of_the_attack_region` playtest covers attack-region access; movement introduces no line-of-sight attack rule. ADR-0016 and ADR-0017 record the navigation and construction hard cutovers.

## Fresh deterministic and rendered verification

Passed:

```text
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test screenshots -- --ignored
cargo build --bin top-down-2d-rts-prototype-nano-swarm --example local_avoidance
python3 scripts/navigation_runtime_acceptance.py --output target/issue-41/runtime
```

Logs are under `target/issue-41/`. The full suite passed 363 unit tests, 518 behavior tests, and 49 scripted playtests. Two GPU playtests and 37 screenshot trials remain ignored in the default invocation; the separate offscreen invocation passed all 37 trials. This documentation follow-up does not introduce a player-facing bug fix, so no new scripted playtest or TDD cycle was necessary; the acceptance map identifies the existing coverage.

The producing agent opened all 22 navigation/lifecycle PNGs below from the fresh run under `target/playtest-screenshots/`:

- `shared_navigation_{before,detour,arrived}`: all three silhouettes begin left of rectangles, pass above their corners with visible clearance, and finish beyond the deposits.
- `local_avoidance_{before,yield,arrived}`: four separate bodies begin in the passage; one retreats above the right wall end; two finish at each side's destination markers, leaving the passage empty.
- `construction_access_{open_passage,safe_alternative}`: the central opening stays clear while an outlined plan appears away from the passage.
- `construction_clearing_{occupied,activated}`: the Defender starts within the planned outline and ends outside the lower edge of the solid Stockpile.
- `construction_cancellation_{pulse,collapse,fade}`: bright red full-size corners shrink and dim behind the Defender without cancellation text.
- `production_exit_{waiting,released}`: eight Workers surround the facility; the south Worker moves away and one yellow Hauler occupies the released exit.
- `route_recovery_{waiting,delivered}`: a loaded Hauler waits left of a full-height wall and then reaches the destination's exterior after removal.
- `exterior_work_{gather,delivery}`, `exterior_defender_charging`, `exterior_hauler_delivery`: working bodies remain visibly outside deposits and structure footprints.
- `physical_logistics`: separated Worker/Hauler bodies, cargo indicators, and four distinct local resource buffers remain visible.

These observations satisfy the corresponding presentation requirements. Exact cargo custody, continuous collision, and once-only output are established by deterministic assertions, not inferred from still frames.

## Fresh real-process acceptance

The existing external driver launched the normal binary with `--headless --agent-socket --width 1280 --height 720` and the offscreen `local_avoidance` example. All actions and synchronization used `scripts/nano_swarm_control.py`; each process used a private mode-0700 runtime directory. No OS/compositor window was created.

All seven fresh runtime PNGs under `target/issue-41/runtime/` were opened individually:

| Capture | Observed facts |
| --- | --- |
| `normal/start` | Four Workers, two Haulers, three Defenders stand outside the authored facility; Corridor is selected and Defender priority is 100%. |
| `normal/working` | Workers surround the deposit; the source and sink supports are solid, with bars; minerals are still zero in this run. |
| `normal/delivery` | HUD shows 196 minerals and three Haulers; another facility is an unfinished outline. |
| `normal/later` | HUD shows 184 minerals, four Haulers, and two facilities; the second facility is solid. Bodies remain outside structures and the match is in progress. |
| `bottleneck/before` | Four distinct bodies occupy the narrow passage between two rectangles. |
| `bottleneck/retreat` | One body backs above the right wall end while three remain separated in the passage. |
| `bottleneck/arrived` | Two bodies reach each side's destination markers; the passage is empty. |

Normal state responses corroborate the mineral/population/facility figures. The deposit HUD falls from 72,000 to 71,736. Snapshot/readback timing and combat outcomes can vary; this run is representative economy integration, not long-term survival proof. The traffic fixture reproduces retreat at tick 23 and arrival at tick 244, with swept separation and structure-clearance assertions passing every fixed tick. Both processes exited zero after client shutdown, removed their sockets, released their advisory locks, and cleaned their runtime directories; `cleanup.json` records each check.

## Performance limits

The benchmark was not rerun for this documentation-only follow-up. The [#66 measurements](issue-66-navigation-scale.md) correspond to the unchanged production implementation at the starting revision, and their retained `target/issue-66/scale-final/` artifacts were confirmed present. They include all four required 5,000-body workloads, hardware/map details, tick percentiles, navigation work, completed-request latency and censored queue backlog, path-cost detours, and a flat-search comparison.

The measurements show reduced long-route search expansions and sampled detours up to 3.834%, but not universally faster wall time. After 600 ticks, the dense workload still has 4,977 pending requests; cold hierarchy construction can exceed flat-search work. These are explicit limitations, not claims of universal throughput or an agreed frame-time guarantee. The issue sets no numerical performance threshold.

## Standards review

Independent review: no findings. Documented testing requirements, linked records, test totals, runtime cleanup evidence, and the distinction between fresh checks and retained measurements were verified. No code-quality concerns were raised for this documentation change.

## Spec review

Independent review: no findings. The reviewer confirmed that closure relies on landed implementation plus acceptance evidence, independently inspected `normal/later.png` and `bottleneck/arrived.png`, and verified the stated benchmark limitations. No missing requirement or scope creep was identified in this follow-up; the review did not rerun tests or exhaustively re-audit every prior implementation commit.

Findings: Standards 0; Spec 0. No unresolved issue on either axis.
