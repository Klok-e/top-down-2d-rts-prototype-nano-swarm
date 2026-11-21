---
status: accepted
---

# AI Battle measures shared autonomy until Swarm Elimination

AI Battle puts two automatically controlled swarms on a balanced map. Their production, logistics, navigation, and combat use the same simulation as Standard. Spectators can move the camera while both swarms retain control of their intent. Scenario definitions select player control, outcome presentation, and statistics collection. The [single-policy decision](0023-one-strategic-controller-policy.md) defines the controller and supported layouts.

Windowed and headless runs share the fixed simulation timestep, seed, and balance configuration. Both run at normal speed by default. Headless execution also supports unbounded pacing. Matching inputs make comparisons more meaningful, although outcomes can vary between runs.

The first Swarm Elimination fixes the outcome and ends statistics collection. Simultaneous elimination is a Draw. A windowed session stays open for inspection and a headless battle exits after saving. An explicit interruption saves the data collected so far. An unfinished battle has no outcome and may continue indefinitely. [Swarm Elimination decision](0020-swarm-elimination-match-outcomes.md)

Each run writes periodic samples and a summary. Population, economy, structures, damage, and tick timing show what the swarms achieved and how performance varied with battle size. The seed, revision, timestep, and tuning identify the workload. [Recording format and operating guide](../ai-battle.md)
