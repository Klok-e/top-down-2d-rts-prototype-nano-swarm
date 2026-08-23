# Regional allocation performance

## Reference machine

- CPU: 13th Gen Intel Core i5-13600KF (20 logical CPUs)
- GPU: AMD Radeon RX 7800 XT, RADV/Mesa 26.1.4, Vulkan
- Benchmark profile: Cargo `bench`, thin LTO
- Framework: Criterion 0.8 with HTML reports
- Command: `cargo bench --bench swarm_acceptance`
- Bot count: 5,000

## Scenarios

`steady_defend_frame` runs 5,000 Defenders against 256 actionable Defend cells after 60 warmup frames. `exhausted_gather_frame` runs 5,000 idle Workers with no actionable resource work, representing the stable state after Gather deposits are exhausted while intent persists. `sparse_distant_gather_frame` runs the same 5,000 Workers after replacing the broad Gather field with one distant owned Gather cell, representing stranded capacity with persistent but remote intent.

## Current result (2026-08-03)

| Scenario | Criterion estimate |
|---|---:|
| Steady Defend | 6.3481–6.6674 ms/frame |
| Exhausted Gather | 5.5464–5.8027 ms/frame |
| Sparse distant Gather | 3.9382–4.1681 ms/frame |

All three scenarios meet the 16.7 ms frame target. Production acquisition partitions projected work by eligible nanobot type, prepares nearest-first region views once per source region, and examines at most 16 regions and 128 opportunities per nanobot. Accepted decisions apply sequentially in stable entity order: each exact claim updates local capacity and regional pull before the next nanobot chooses. Defend claims remain soft-overcapacity, while exclusive Planned Build and Maintenance claims use current ECS reservations as conflict-aware eligibility. The latest Criterion run improved steady Defend by 14.5%, exhausted Gather by 12.0%, and sparse distant Gather by 7.3% against their stored baselines; all three changes were statistically significant. Compared with the earlier recorded 9.1072–9.2865 ms steady-Defend range, the current midpoint is approximately 29.2% lower. All estimates remain well below the frame budget.

Criterion HTML reports are generated under `target/criterion/`.
