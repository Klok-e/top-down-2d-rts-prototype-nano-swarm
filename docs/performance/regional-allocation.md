# Regional allocation performance

## Reference machine

- CPU: 13th Gen Intel Core i5-13600KF (20 logical CPUs)
- GPU: AMD Radeon RX 7800 XT, RADV/Mesa 26.1.4, Vulkan
- Benchmark profile: Cargo `bench`, thin LTO
- Framework: Criterion 0.8 with HTML reports
- Command: `cargo bench --bench swarm_acceptance`
- Bot count: 5,000

## Scenarios

`steady_threat_response_frame` runs 5,000 Defenders split evenly between two swarms across 256 player-owned Swarm Tiles after 60 warmup frames. The 2,500 hostile Defenders are live Threats, so the case measures territory projection, bounded response reconciliation, movement, and combat rather than deleted stable-holder work. `exhausted_gather_frame` runs 5,000 idle Workers with no actionable resource work, representing the stable state after Gather deposits are exhausted while intent persists. `sparse_distant_gather_frame` runs the same 5,000 Workers after replacing the broad Gather field with one distant owned Gather cell, representing stranded capacity with persistent but remote intent.

## Current result (2026-08-30)

| Scenario | Criterion estimate |
|---|---:|
| Steady Threat response | 9.2706–9.4683 ms/frame |
| Exhausted Gather | 6.0875–6.3239 ms/frame |
| Sparse distant Gather | 4.3985–4.4829 ms/frame |

All three scenarios meet the 16.7 ms frame target. Production acquisition partitions projected work by eligible nanobot type, prepares nearest-first region views once per source region, and examines at most 16 regions and 128 opportunities per nanobot. Threat response prepares one mutable unclaimed-work index per fixed step, partitions work by danger tier before applying proximity bounds, removes exhausted regions as claims are acquired, and examines at most 16 work regions and 128 Threat candidates per Defender while preserving one response per Threat. The steady result is not directly comparable with the deleted `steady_defend_frame` baseline because the new case exercises materially different territory, response, movement, and combat work. Against the immediately preceding indexed implementation, Criterion classified the steady and sparse cases as small regressions and detected no statistically significant exhausted Gather change.

Criterion HTML reports are generated under `target/criterion/`.
