# Regional allocation performance

## Reference machine

- CPU: 13th Gen Intel Core i5-13600KF (20 logical CPUs)
- GPU: AMD Radeon RX 7800 XT, RADV/Mesa 26.1.4, Vulkan
- Benchmark profile: Cargo `bench`, thin LTO
- Framework: Criterion 0.8 with HTML reports
- Command: `cargo bench --bench swarm_acceptance`
- P95 proof command: `taskset -c 0 env NANO_SWARM_ACCEPTANCE_PROOF_ONLY=1 cargo bench --bench swarm_acceptance -- --noplot`
- Bot count: 5,000

## Scenarios

`steady_threat_response_frame` runs 5,000 Defenders split evenly between two swarms across 256 player-owned Swarm Tiles after 60 warmup frames. The 2,500 hostile Defenders are live Threats, so the case measures territory projection, bounded response reconciliation, movement, and combat rather than deleted stable-holder work. `unengaged_staging_frame` runs 5,000 same-swarm Defenders with one owned Defend cell and no Threats, so every Defender exercises cached staging plus continuous movement and separation. Its acceptance setup then adds an eastern Defend cell alongside the western cell and times a full-cohort redistribution to 2,500 Defenders per cell. `exhausted_gather_frame` runs 5,000 idle Workers with no actionable resource work, representing the stable state after Gather deposits are exhausted while intent persists. `sparse_distant_gather_frame` runs the same 5,000 Workers after replacing the broad Gather field with one distant owned Gather cell, representing stranded capacity with persistent but remote intent.

## Current result (2026-08-31)

| Scenario | Criterion estimate |
|---|---:|
| Steady Threat response | 5.1992–5.2072 ms/frame |
| Unengaged staging | 5.7566–6.0003 ms/frame |
| Exhausted Gather | 2.8399–2.8749 ms/frame |
| Sparse distant Gather | 1.3082–1.3179 ms/frame |

All four scenarios meet the 16.7 ms frame target. Production acquisition partitions projected work by eligible nanobot type, prepares nearest-first region views once per source region, and examines at most 16 regions and 128 opportunities per nanobot. Empty general projections skip candidate and fairness bookkeeping. Threat response validates existing claims and movement in place, returns before constructing uncovered-work and preemption indexes when every Threat is covered, and otherwise examines at most 16 work regions and 128 Threat candidates per Defender while preserving one response per Threat. Stable staging layouts reuse their owner-scoped cohort and target-cell assignment while procedural roaming continues every fixed tick. A cohort or effective-cell change preserves exact balance and lexicographically minimizes incumbent relocation, then current-cell relocation; small cohorts minimize total travel exactly after those continuity constraints, while large cohorts choose remaining travel from at most 128 axis-local candidates per Defender. The one-cell case retargets directly in linear time. Fixed spatial buckets use Bevy's deterministic fixed-hash map while retaining explicit traversal and per-bucket sorting. The steady Threat result is not directly comparable with the deleted `steady_defend_frame` baseline because the new case exercises materially different territory, response, movement, and combat work.

## Explicit p95 acceptance proof (2026-08-31)

The benchmark warms up for 60 frames, records 600 explicit samples, and uses nearest-rank p95. The allocation interval covers the chained `Project`, `Invalidate`, and `Acquire` sets; the separation interval covers only the local separation system. The latest pinned-core full benchmark run recorded:

| Metric | P95 | Budget |
|---|---:|---:|
| Whole frame | 5.2553 ms | 16.7000 ms |
| Regional allocation, combined | 1.5022 ms | 2.0000 ms |
| Project | 0.6883 ms | — |
| Invalidate | 0.0220 ms | — |
| Acquire | 0.8079 ms | — |
| Local separation | 1.3175 ms | 3.0000 ms |

The same pinned run redistributed all 5,000 unengaged Defenders after adding a second owned Defend cell: the edit frame took 2.8299 ms against the 16.7000 ms frame budget, and combined Project/Invalidate/Acquire took 1.6431 ms against the 2.0000 ms allocation budget. The observable entity-to-cell assignment balanced 2,500 Defenders per cell and stayed unchanged on the following fixed step. These values are latency measurements, not Criterion confidence intervals.

Criterion HTML reports are generated under `target/criterion/`.
