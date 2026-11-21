# Strategic Controller verification

Standard's opponent and both AI Battle swarms use the same Strategic Controller. Its observable contract spans the intent it owns and the work its Nanobots carry out. [Single-policy decision](../adr/0023-one-strategic-controller-policy.md)

| Behavior | Evidence |
| --- | --- |
| Each swarm edits only its own paint. Cleanup eventually reaches stale owned intent even behind a long prefix of foreign cells. | [Owner isolation and cleanup regressions](../../tests/behavior/strategic_controller.rs) |
| Once the match finishes, the controller skips its review and emits no edits. | [Controller test for a finished match](../../src/strategic_controller/tests.rs) |
| Depleting one Owned Deposit triggers a review while the other mines keep their Gather, Build, Defend, and Corridor paint. Route searches and urgent reviews use the same bounded allowance as regular reviews. | [Controller decisions on snapshots](../../src/strategic_controller/tests.rs) |
| Workers extract and Haulers move minerals from several supported deposits. On an obstructed map, the Logistics Corridor connects a deposit to home around the rock. | [Economy and navigation behavior tests](../../tests/behavior/strategic_controller.rs) |
| The military front holds its Staging Point until enough living Defenders arrive. When the plan changes, the controller erases superseded paint in the same review. | [Staging and retargeting playtests](../../tests/playtest/opponent_gameplay_loop.rs) |
| Defend paint grows with income and supplied Chargers. Depleted reserves reduce the area and real Defender Population Demand. | [Spending tests](../../src/strategic_controller/tests.rs) and [Population Demand behavior](../../tests/behavior/strategic_controller.rs) |
| Busy production and sufficient physical reserves open more Build room near the economy. The same room supports recovery after the last facility is lost. | [Production Pressure behavior](../../tests/behavior/strategic_controller.rs) |

The controller estimates upkeep from recent physical extraction, operational mineral buffers, and supported Charger capacity. Required paint for the economy and active staging point sets a lower bound on territory. Production Pressure considers zero operational facilities saturated for recovery, while an idle or output-blocked facility prevents saturation. Shared placement ranks feasible sites across all owned Build Zones, including Build paint that supports logistics at the front.

## Observed self-play

The economy comparison uses seed 11, the tracked balance configuration, unbounded headless simulation, and one-second samples. Each opening covers the same first 7,200 fixed ticks. An active mine in the tables means a distinct deposit that yielded minerals under the swarm's Gather paint during that sample. A mine can keep its paint between extraction pulses. Starvation is summed across facilities, so two waiting facilities can accumulate two facility-seconds in one second of simulation. [AI Battle recording guide](../ai-battle.md)

The telemetry baseline used commit `4a7c7e5`, which still used the single-deposit controller. Both layouts ran for 120 simulated seconds without recording combat damage. The peak active mine count was one for each swarm on each layout.

| Layout | Swarm | Minerals extracted | Mean unspent | Starved facility-seconds | Peak empty-Charge Defenders |
| --- | ---: | ---: | ---: | ---: | ---: |
| Standard | 0 | 277 | 123.4 | 217.5 | 0 |
| Standard | 1 | 301 | 119.3 | 202.1 | 0 |
| Flanks | 0 | 236 | 115.7 | 167.6 | 0 |
| Flanks | 1 | 274 | 119.4 | 90.3 | 0 |

## Earlier self-play

The September 2026 single-policy verification recorded two natural eliminations with seed 11. Swarm 1 won on Standard after 27,165 fixed ticks (452.75 simulated seconds) and on Flanks after 17,665 ticks (294.42 seconds). These runs show that both layouts could reach an outcome with that revision and tuning. Their times and winners are historical observations.

During a normal-speed Standard observation, the opponent expanded toward the center. In a separate normal-speed AI Battle, both swarms gathered, built structures, and dealt damage. An explicit shutdown recorded the still-active battle as interrupted with no outcome. The original observations are recorded in Git at `30419d7:docs/verification/strategic-controller.md`. The [AI Battle guide](../ai-battle.md) describes how to run self-play and interpret new recordings.

Actual novice difficulty remains unverified by human play in Standard.
