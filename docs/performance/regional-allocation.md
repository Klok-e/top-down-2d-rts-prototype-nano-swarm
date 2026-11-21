# Regional allocation performance

Run the four 5,000-Nanobot workloads with `cargo bench --bench swarm_acceptance`. Criterion writes its reports under `target/criterion/`. [Benchmark source](../../benches/swarm_acceptance.rs)

| Workload | What it exercises |
| --- | --- |
| `steady_threat_response_frame` | 2,500 Defenders respond to 2,500 hostile Defenders on owned territory. |
| `unengaged_staging_frame` | 5,000 friendly Defenders stage, move, and separate around one Defend cell with no Threats. |
| `exhausted_gather_frame` | 5,000 Workers have a broad Gather Zone with no actionable deposits. |
| `sparse_distant_gather_frame` | The same idle workforce has a single distant Gather cell. |

## Acceptance budgets

To run just the acceptance checks, use `NANO_SWARM_ACCEPTANCE_PROOF_ONLY=1 cargo bench --bench swarm_acceptance`. Use the same machine and CPU affinity when comparing revisions.

The Threat response check warms up for 60 frames and takes the nearest-rank p95 across 600 samples. The enforced budgets are 16.7 ms for the whole frame, 2 ms for allocation across Project, Invalidate, and Acquire together, and 3 ms for local separation.

A separate single-frame check adds a second Defend cell during unengaged staging. It requires 2,500 Defenders per cell, stable assignments on the next update, and the same frame and allocation budgets on the edit itself.

## Historical baseline

On 2026-08-31, a pinned-core run on an Intel Core i5-13600KF measured p95 of 5.2553 ms for the Threat response frame, 1.5022 ms for allocation, and 1.3175 ms for separation. Adding the second Defend cell took 2.8299 ms, including 1.6431 ms of allocation.
