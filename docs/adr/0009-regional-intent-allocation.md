# Regional intent allocation

Nanobot work allocation uses dirty-region projection from persistent Intent Zones into actionable opportunities, deterministic fixed spatial buckets, and regional category-capacity leases. Allocation runs at 10 Hz with same-tick invalidation of changed work; regional pull budgets provide Global Intent Awareness, while nanobots make bounded local choices and claim exact Soft Work Slots only when committing to a target. This deliberately replaces per-nanobot global intent scans: approximate local decisions preserve Dumb Autonomy while making 5,000 nanobots feasible.

## Consequences

- Intent remains authoritative and persistent. Derived actionable opportunities are recomputed for dirty regions from intent and ECS state; lifecycle systems only invalidate regions and do not own the projection.
- Valid distant work attracts capacity through regional pull budgets. Minimum Category Activation is satisfied before remaining capacity is distributed by weighted pressure.
- Binary intent contributes eligibility, not a pressure multiplier. Gather and Build pressure comes from useful work; each Defend cell contributes one baseline Soft Work Slot. Players shape allocation through zone geometry rather than paint intensity or task-priority controls.
- Regional leases renew while progress is measurable, expire after no progress, and are revoked immediately only when supporting work becomes invalid. Rebalancing otherwise uses idle nanobots and expired leases.
- Reassignment bursts are bounded by a per-region percentage with a small floor.
- Independent regions may be processed in parallel, but allocation order, bot identity tie-breakers, and merged results remain deterministic.
- Fixed spatial indexing is shared as an abstraction, not as one coupled index or bucket size. Work allocation and local separation keep distinct typed contents and resolutions.
- Acceptance uses two repeatable 5,000-nanobot scenarios on the recorded development machine: mixed steady work and simultaneous Resource Deposit exhaustion with persistent Gather Zones. Target p95 frame time is 16.7 ms, allocation ticks at most 2 ms, local separation at most 3 ms, and depletion frames below 33 ms.
- Migration replaces allocation for all work categories together rather than maintaining legacy and regional authorities side by side. The codebase is small enough that one coherent cutover has lower conceptual cost than staged coexistence, despite higher short-term regression risk.
- Regional allocation owns Gather, Planned Build, Maintenance, Defend, and Haul job acquisition. Production decisions, structure creation, movement, and combat remain separate. Emergency Charge overrides normal allocation: a low-charge Defender suspends its lease without satisfying active capacity, allowing a temporary replacement; it resumes only while capacity remains valid.
- Haul opportunities are anchored in source regions because job acquisition requires available pickup material. Destination pressure, downstream priority, and full Logistics Leg route cost still contribute to opportunity ranking.
- Legacy allocators are deleted after replacement behavior and performance tests pass; version control provides rollback rather than dormant feature-flagged architecture.
