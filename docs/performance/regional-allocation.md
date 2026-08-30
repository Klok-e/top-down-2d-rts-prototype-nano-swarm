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

`steady_threat_response_frame` runs 5,000 Defenders split evenly between two swarms across 256 player-owned Swarm Tiles after 60 warmup frames. The 2,500 hostile Defenders are live Threats, so the case measures territory projection, bounded response reconciliation, movement, and combat rather than deleted stable-holder work. `exhausted_gather_frame` runs 5,000 idle Workers with no actionable resource work, representing the stable state after Gather deposits are exhausted while intent persists. `sparse_distant_gather_frame` runs the same 5,000 Workers after replacing the broad Gather field with one distant owned Gather cell, representing stranded capacity with persistent but remote intent.

## Current result (2026-08-30)

| Scenario | Criterion estimate |
|---|---:|
| Steady Threat response | 5.5710–5.6206 ms/frame |
| Exhausted Gather | 3.0918–3.2000 ms/frame |
| Sparse distant Gather | 1.4568–1.4748 ms/frame |

All three scenarios meet the 16.7 ms frame target. Production acquisition partitions projected work by eligible nanobot type, prepares nearest-first region views once per source region, and examines at most 16 regions and 128 opportunities per nanobot. Empty general projections skip candidate and fairness bookkeeping. Threat response validates existing claims and movement in place, returns before constructing uncovered-work and preemption indexes when every Threat is covered, and otherwise examines at most 16 work regions and 128 Threat candidates per Defender while preserving one response per Threat. Fixed spatial buckets use Bevy's deterministic fixed-hash map while retaining explicit traversal and per-bucket sorting. The steady result is not directly comparable with the deleted `steady_defend_frame` baseline because the new case exercises materially different territory, response, movement, and combat work. Criterion measured improvements of 36.5–37.5% for steady Threat response, 36.4–38.5% for exhausted Gather, and 56.7–57.8% for sparse distant Gather against the preceding indexed implementation.

## Explicit p95 acceptance proof (2026-08-30)

The benchmark warms up for 60 frames, records 600 explicit samples, and uses nearest-rank p95. The allocation interval covers the chained `Project`, `Invalidate`, and `Acquire` sets; the separation interval covers only the local separation system. The latest pinned-core full benchmark run recorded:

| Metric | P95 | Budget |
|---|---:|---:|
| Whole frame | 5.7693 ms | 16.7000 ms |
| Regional allocation, combined | 1.7430 ms | 2.0000 ms |
| Project | 0.8091 ms | — |
| Invalidate | 0.0316 ms | — |
| Acquire | 0.9604 ms | — |
| Local separation | 1.4443 ms | 3.0000 ms |

Two immediately preceding proof-only runs also passed: combined allocation was 1.7083 ms and 1.6795 ms, while local separation was 1.4378 ms and 1.4480 ms. These values are latency percentiles, not Criterion confidence intervals.

Criterion HTML reports are generated under `target/criterion/`.
