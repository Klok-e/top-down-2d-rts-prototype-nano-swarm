# Regional allocation performance

## Reference machine

- CPU: 13th Gen Intel Core i5-13600KF (20 logical CPUs)
- GPU: AMD Radeon RX 7800 XT, RADV/Mesa 26.1.4, Vulkan
- Benchmark profile: Cargo `bench`, thin LTO
- Framework: Criterion 0.8 with HTML reports
- Command: `cargo bench --bench swarm_acceptance`
- Bot count: 5,000

## Scenarios

`steady_defend_frame` runs 5,000 Defenders against 256 actionable Defend cells after 60 warmup frames. `exhausted_gather_frame` runs 5,000 idle Workers with no actionable resource work, representing the stable state after Gather deposits are exhausted while intent persists.

## Current result

| Scenario | Criterion estimate |
|---|---:|
| Steady Defend | 9.1072–9.2865 ms/frame |
| Exhausted Gather | 6.5263–6.7062 ms/frame |

Both scenarios meet the 16.7 ms frame target. Production acquisition partitions projected work by eligible nanobot type, prepares nearest-first region views once per source region, and examines at most 16 regions and 128 opportunities per nanobot. Accepted decisions apply sequentially in stable entity order: each exact claim updates local capacity and regional pull before the next nanobot chooses. Defend claims remain soft-overcapacity, while exclusive Planned Build and Maintenance claims use current ECS reservations as conflict-aware eligibility. Values above come from the final verification run; normal machine-load variance explains differences from earlier Criterion samples.

Criterion HTML reports are generated under `target/criterion/`.
