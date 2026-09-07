# Swarm elimination determines match outcomes

Replace Production Collapse with Swarm Elimination: a swarm is eliminated only when it has zero Nanobots and zero completed structures, with Planned Structures excluded. Any surviving Nanobot, Production Facility, Stockpile, or Charger keeps the swarm in the match regardless of production, supply, or recovery potential; this makes the outcome predictable while accepting that a hopeless swarm can prolong a match until its remaining entities are destroyed or decay.

In Standard, evaluate both swarms together at the end of each simulation tick after entity creation and removal are reflected in the state: only the opponent eliminated means Victory, only the player eliminated means Defeat, and both eliminated in that tick means Draw. The first result is permanent even if later simulation state changes; Sandbox has no match outcomes.

Make a hard cutover by removing Production Collapse and its recovery-path checks, including obsolete terminology and outcome-specific interfaces, presentation, and tests. This decision replaces recovery prediction as a match-ending rule.
