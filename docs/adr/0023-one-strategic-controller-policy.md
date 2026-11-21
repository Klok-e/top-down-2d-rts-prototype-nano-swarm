---
status: accepted
---

# Use one Strategic Controller policy

Standard's Opponent Swarm and both sides of AI Battle use one Strategic Controller. Maintaining a policy selector, frozen opponents, and a promotion ladder encouraged optimizing comparative scores. One shared policy lets a failure observed in self-play exercise the same decision logic a player faces. [Novice-opponent objective](0022-strategic-controller-objective.md)

The controller takes an immutable game-state snapshot and emits explicit Paint or Erase actions for its own swarm. Planning and cleanup share a bounded budget, and cleanup resumes across reviews so earlier foreign paint cannot hide stale owned intent. Nanobots execute plans through shared autonomy. Planning stops when a match finishes.

AI Battle provides symmetric self-play on Standard and Flanks. Both sides share the balance configuration and the recorder tracks effective damage alongside population, economy, and structures. Review counts, intent edits, work units, and the latest explanation help connect observed behavior to a controller decision. The [AI Battle decision](0021-ai-battle-benchmark.md) defines the recording boundary.

The [verification record](../verification/strategic-controller.md) covers the shipped policy. The [research archive](../verification/strategic-controller-research-history.md) preserves the rationale and negative results from competitive experiments.
