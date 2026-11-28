# Strategic Controller verification

Standard's opponent and both AI Battle swarms use the same Strategic Controller. Its observable contract spans the intent it owns and the work its Nanobots carry out. [Single-policy decision](../adr/0023-one-strategic-controller-policy.md)

| Behavior | Evidence |
| --- | --- |
| Each swarm edits only its own paint. Cleanup eventually reaches stale owned intent even behind a long prefix of foreign cells. | [Owner isolation and cleanup regressions](../../tests/behavior/strategic_controller.rs) |
| Once the match finishes, the controller skips its review and emits no edits. | [Controller test for a finished match](../../src/strategic_controller/tests.rs) |
| Depleting one Owned Deposit triggers a review while the other mines keep their Gather, Build, Defend, and Corridor paint. Route searches and urgent reviews use the same bounded allowance as regular reviews. | [Controller decisions on snapshots](../../src/strategic_controller/tests.rs) |
| Workers extract and Haulers move minerals from several supported deposits. On an obstructed map, the Logistics Corridor connects a deposit to home around the rock. | [Economy and navigation behavior tests](../../tests/behavior/strategic_controller.rs) |
| Credible attacks and sustained hostile disruption can redirect the military front to a mine. Immediate danger and recent mine output determine the response. Local Defenders handle a passing Worker. | [Mine defense behavior](../../tests/behavior/strategic_mine_defense.rs) and [local defense playtest](../../tests/playtest/strategic_local_defense.rs) |
| A raid holds its Staging Point through a brief readiness loss. Advancement requires readiness and enough assigned Defenders in the active Defend block. Assigned Defenders awaiting routes count toward the arrival fraction. | [Staging and retargeting playtests](../../tests/playtest/opponent_gameplay_loop.rs) |
| Defend paint grows with income and supplied Chargers. Depleted reserves reduce the area and real Defender Population Demand. | [Spending tests](../../src/strategic_controller/tests.rs) and [Population Demand behavior](../../tests/behavior/strategic_controller.rs) |
| Committed structures keep their reachable Build footprints through construction, idle production, and Staging Point moves. Home and retained mines keep a small allowance of spare Build room for Production Pressure. | [Controller behavior](../../tests/behavior/strategic_controller.rs) and [structure paint playtests](../../tests/playtest/structure_paint.rs) |
| Normal expansion follows sustained Worker activity and completed drop-offs at retained mines, a supply gap that more extraction can close, and spare Worker and Hauler capacity. An eligible expansion can share the plan with a raid within the work and paint budgets. | [Controller behavior](../../tests/behavior/strategic_controller.rs) and [opening and raid playtests](../../tests/playtest/opponent_gameplay_loop.rs) |

The controller estimates upkeep from recent physical extraction, operational mineral buffers, and supported Charger capacity. Required paint for the economy and active Staging Point sets a lower bound on territory. Production Pressure considers zero operational facilities saturated for recovery, while an idle or output-blocked facility prevents saturation. Shared placement ranks feasible sites across owned Build Zones.

## Observed self-play

The economy comparison uses seed 11, the tracked balance configuration, unbounded headless simulation, and one-second samples. Each opening covers the same first 7,200 fixed ticks. An active mine in the tables means a distinct deposit that yielded minerals under the swarm's Gather paint during that sample. A mine can keep its paint between extraction pulses. Starvation is summed across facilities, so two waiting facilities can accumulate two facility-seconds in one second of simulation. [AI Battle recording guide](../ai-battle.md)

The baseline used the single-deposit controller at `4a7c7e5`. Both layouts ran for 120 simulated seconds without combat damage or structure losses. No buffers were lost during the comparison window, so the baseline's ledger and the revised physical inventory measure the same unspent minerals.

Each pair shows the baseline followed by the expansion controller at `d2f7d4b`.

| Layout | Swarm | Peak extracting deposits | Extracted minerals | Mean unspent minerals | Starved facility-seconds |
| --- | ---: | ---: | ---: | ---: | ---: |
| Standard | 0 | 1 → 2 | 277 → 132 | 123.4 → 44.2 | 217.5 → 137.6 |
| Standard | 1 | 1 → 2 | 301 → 170 | 119.3 → 57.8 | 202.1 → 132.0 |
| Flanks | 0 | 1 → 2 | 236 → 148 | 115.7 → 35.5 | 167.6 → 119.9 |
| Flanks | 1 | 1 → 2 | 274 → 200 | 119.4 → 56.2 | 90.3 → 110.8 |

All four swarms extracted from two deposits within one sample before major combat. Each kept fewer minerals unspent and also extracted fewer minerals in the first 120 seconds. Combined facility starvation fell on both layouts. Flanks swarm 1 spent 20.5 more facility-seconds waiting for minerals. No sampled Defender reached empty Charge during either opening in either revision.

The outcome comparison defines the first major fight as the first sample in which cumulative effective damage by both sides reached 100, the health of one Nanobot. Pre-fight income counts minerals extracted over the preceding 30 simulated seconds. Standard swarm 0 led 14 to 4 before its fight at 155 seconds, although swarm 1 had extracted more since the start (176 to 148). Flanks swarm 1 led 65 to 58 before its fight at 301 seconds and also led in cumulative extraction (518 to 387). Both sides reached the same peak of two extracting sites before combat on each layout, so these games do not distinguish a better expander by that measure.

| Layout | Pre-fight income leader | Population at shutdown, swarm 0 / 1 | Defender observations at empty Charge, swarm 0 / 1 | Outcome |
| --- | --- | ---: | ---: | --- |
| Standard | Swarm 0 | 20 / 2 | 11.36% / 6.10% | In progress at 30 minutes |
| Flanks | Swarm 1 | 7 / 49 | 4.22% / 4.37% | In progress at 30 minutes |

The side with the pre-fight income lead had the larger surviving population on both layouts. Neither game reached Swarm Elimination before shutdown, so neither established that the income leader wins. The Charge percentages divide all empty-Defender observations by all Defender observations across the full run. Standard swarm 0 spent the largest share of its observed Defender time empty.

The Standard capture at 120 seconds shows compact blue fronts in the interior and a yellow supply route skirting the southern rock. At 200 seconds on Flanks, the yellow approach reaches a blue block beside the long central rock. In a separate controlled advance, two of four living Defenders held a 4×3 block at `(3, 0)`. When the third arrived, the block moved to `(6, 0)` in one review. All nine cells exclusive to the previous block lost their blue Defend hatching. The yellow Logistics Corridor remained visible. [Staging and erasure playtests](../../tests/playtest/opponent_gameplay_loop.rs)

## Earlier self-play

The September 2026 single-policy verification recorded two natural eliminations with seed 11. Swarm 1 won on Standard after 27,165 fixed ticks (452.75 simulated seconds) and on Flanks after 17,665 ticks (294.42 seconds). These runs show that both layouts could reach an outcome with that revision and tuning. Their times and winners are historical observations.

During a normal-speed Standard observation, the opponent expanded toward the center. In a separate normal-speed AI Battle, both swarms gathered, built structures, and dealt damage. An explicit shutdown recorded the still-active battle as interrupted with no outcome. The original observations are recorded in Git at `30419d7:docs/verification/strategic-controller.md`. The [AI Battle guide](../ai-battle.md) describes how to run self-play and interpret new recordings.

Actual novice difficulty remains unverified by human play in Standard.
