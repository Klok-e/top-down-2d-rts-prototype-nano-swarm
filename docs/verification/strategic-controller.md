# Strategic Controller verification

Standard's opponent and both AI Battle swarms use the same Strategic Controller. Its observable contract spans the intent it owns and the work its Nanobots carry out. [Single-policy decision](../adr/0023-one-strategic-controller-policy.md)

| Behavior | Evidence |
| --- | --- |
| Each swarm edits only its own paint. Cleanup eventually reaches stale owned intent even behind a long prefix of foreign cells. | [Owner isolation and cleanup regressions](../../tests/behavior/strategic_controller.rs) |
| Once the match finishes, the controller skips its review and emits no edits. | [Controller test for a finished match](../../src/strategic_controller/tests.rs) |
| Depleting the primary Resource Deposit or losing a key Charger triggers replanning. Urgent and regular reviews draw on the same work allowance. | [Resource, support, and budget regressions](../../tests/behavior/strategic_controller.rs) |
| A primary resource site supports extraction and hauling. Starting a second front can preserve a Defender response already making progress. | [Economy and front-continuity tests](../../tests/behavior/strategic_controller.rs) |
| Combat support follows advancing Defenders. Shared combat can destroy a Production Facility that is still producing reinforcements and reach a survivor away from its former home. | [Opponent gameplay playtests](../../tests/playtest/opponent_gameplay_loop.rs) |

## Observed self-play

The September 2026 single-policy verification recorded two natural eliminations with seed 11. Swarm 1 won on Standard after 27,165 fixed ticks (452.75 simulated seconds) and on Flanks after 17,665 ticks (294.42 seconds). These runs show that both layouts could reach an outcome with that revision and tuning. Their times and winners are historical observations.

During a normal-speed Standard observation, the opponent expanded toward the center. In a separate normal-speed AI Battle, both swarms gathered, built structures, and dealt damage. An explicit shutdown recorded the still-active battle as interrupted with no outcome. The original observations are recorded in Git at `30419d7:docs/verification/strategic-controller.md`. The [AI Battle guide](../ai-battle.md) describes how to run self-play and interpret new recordings.

Actual novice difficulty remains unverified by human play in Standard.
