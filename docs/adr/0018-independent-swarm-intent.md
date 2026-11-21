---
status: accepted
---

# Independent swarm intent

Paint expresses a swarm's orders rather than an exclusive claim to space. Every intent belongs to a specific swarm, and multiple swarms may independently paint the same kind at the same location. This replaces Defend Contests and unowned paint with one ownership rule: painting or erasing changes only the acting swarm's intent, and combat never captures, transfers, or erases paint. Enemy orders persist after their Defenders are eliminated.

This is a hard cutover: remove contest state, presence-based resolution, capture timers, and shared unowned paint semantics. Defender engagement follows [ADR 0025](0025-defender-engagement.md). This decision supersedes the unowned-paint rule in [ADR-0009](0009-regional-intent-allocation.md), and refines [ADR-0010](0010-binary-intent-paint.md) so binary intent is scoped by swarm as well as kind and location.

## Consequences

- Gather, Build, Defend, and Corridor intent follow the same independent ownership rule. Every paint kind establishes a Swarm Tile for its owner, and an overlapping cell counts once for each participating swarm. Hostile nanobots and structures there are Threats under ADR-0025, including when Gather or Build paint is placed over an enemy base.
- Swarms with overlapping Gather paint may harvest the same deposit. Actual extraction consumes its single finite resource pool; paint reserves no share and grants no exclusive access.
- Swarms with overlapping Build paint may construct in physically free space. Structures and construction reservations block placement regardless of owner. Paint alone reserves no space, and enemy structures remain enemy-owned.
- Defend staging and Charger eligibility use each swarm's own Defend paint. Overlap does not neutralize either swarm's orders. Corridor guidance likewise uses the travelling swarm's own paint.
- Each intent keeps its own color. When both swarms paint a cell, the overlay gives each swarm a separate row and shows every intent it owns. The player sees solid fill for their orders and hatching for the opponent’s. AI Battle uses opposite hatch directions for its two swarms.

Independent overlap is preferred to blocking enemy paint or retaining shared unowned intent because each swarm can express its orders without editing another swarm's orders. Competition occurs through physical combat, finite resources, and occupied construction space.
