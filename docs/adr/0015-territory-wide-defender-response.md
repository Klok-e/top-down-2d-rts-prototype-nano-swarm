---
status: accepted
---

# Territory-wide Defender response with Defend staging

Defenders protect all Swarm Tiles rather than holding and fighting only inside individually leased Defend cells. Defend Zones are positioning intent for unengaged Defenders: they stage at equal density, rebalance on paint changes, and continuously roam through density-driven cross-cell movement and deterministic procedural subcell waypoints. This supersedes [ADR 0007](./0007-defender-spatial-pressure.md) and the Defender-specific lease consequences of [ADR 0009](./0009-regional-intent-allocation.md); regional projection, bounded local choice, and deterministic outcomes remain authoritative.

Every hostile nanobot or structure physically present on a Swarm Tile is a Threat. Each Threat creates at most one pursuit response, with deterministic bounded-nearest matching, priority from hostile Defender to other nanobot to structure, and preemption only from a higher priority tier. Response assignments control movement but not exclusive attack ownership: a responding Defender attacks the nearest hostile within range. A target that leaves territory remains valid only through the one-cell Pursuit Halo, including diagonal neighbors; Defender transit itself is unrestricted. Defender destinations react to Threat and Defend-paint changes on the next fixed simulation step.

Defender Population Demand is the greater of half the swarm's unique Swarm Tile count, rounded up, and its active Threat count. A cell counts once for each swarm that claims it regardless of overlapping owned layers. Defend paint creates no additional Defender Population Demand; it only distributes the available unengaged cohort.

Unengaged staging layouts distribute Defenders equally per Defend cell and minimize cohort travel when several balanced layouts exist. Cross-cell redistribution uses normal movement speed; local roaming is gentle and continuous. Without Defend paint, Defenders stage across Swarm Tiles, and without any Swarm Tiles they roam procedurally inside their current cells. Any living Defender physically present in a contested Defend cell counts for its swarm's contest presence.

Charge no longer preserves cell or response ownership. Low-charge Defenders select the nearest valid supplied Charger in any owned Defend Zone, subject to the three-user Charger cap and a swarm-wide rotation cap of half the living Defender population, rounded down with a minimum of one. Lowest Charge receives scarce rotation capacity first; ties prefer staged Defenders, then stable identity. A Defender without valid available capacity continues its current duty while weakening. Rotation immediately releases a response for replacement, and recharged Defenders re-enter current response or staging allocation without reclaiming prior work.

Charger construction responds only when a low-charge Defender lacks available valid capacity and uses the nearest eligible non-overlapping site in owned Defend paint. A completed Charger outside owned Defend paint remains standing but is inactive. A valid Charger creates Maintenance demand only while actively serving a Defender or while a living same-swarm Defender is in its cell or an adjacent cell, including diagonals; unattended capacity may decay.

## Consequences

- Stable Defend-cell holder leases and same-cell Charger cohorts are removed rather than retained as a parallel authority.
- Painting any owned intent layer establishes territory defense; cells with layers owned by different swarms are Swarm Tiles for both and create mutual Threats.
- Defend paint controls where idle defense mass waits and where Chargers may operate, not the boundary of protected territory or the size of the Defender population.
- Combat cadence, damage, Charge thresholds and economics, the movement-speed cap, and non-Defender allocation remain unchanged.
