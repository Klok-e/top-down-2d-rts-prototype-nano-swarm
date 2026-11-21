# Archived Strategic Controller experiments

Before Nano Swarm adopted one Strategic Controller, experiments compared an adaptive candidate with a timed policy and frozen snapshots of earlier candidates. Trials used both starting sides and stopped after 600 simulated seconds. A trial that reached the time limit was unresolved. The [single-policy decision](../adr/0023-one-strategic-controller-policy.md) explains why the project retired competitive evaluation.

| Experiment | Observed result and interpretation |
| --- | --- |
| Finishing a distant survivor (v6) | A newly completed enemy structure drew Defend paint away from a front already making progress. In the swapped-start trial, one enemy Hauler survived to the cutoff. These observations motivated tests for retaining a useful front while opening another. |
| Expanding to a second Resource Deposit (v8–v9) | Workers travelled roughly 4,000 units to the home Source Stockpile before the outlying Source was completed. Some later long trips coincided with a nearly full outlying buffer. Delaying expansion until the primary economy held minerals delayed the first expansion and improved early gathering in one repeat. That short run left the effect on winning unresolved. |
| Targeting by current health (v12) | Removing target-class preferences and favoring immediately damageable health produced zero wins, one loss, and three unresolved trials for the candidate on Standard, seed 11. In the swapped start against the timed opponent, its credited damage rose from 4,348 in v11 to 8,644 in v12, but the earlier version won and v12 timed out. The experiment was rejected. Greater damage could coexist with worse finishing. |

The experiments capped credited damage at one health bar per enemy lifetime to limit repeat credit from repairs. AI Battle now records actual effective damage. No held-out experiment established a difficulty ranking or a policy promotion. The full pilot matrices and their evaluation criteria are preserved in Git at `30419d7:docs/verification/strategic-controller-research-history.md`. Evidence for the shipped controller lives in the [verification record](strategic-controller.md).
