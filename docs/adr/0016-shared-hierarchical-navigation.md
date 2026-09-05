# Shared hierarchical navigation

All Nanobot Types use shared hierarchical pathfinding over a separate fine grid whose cells fit approximately one nanobot body plus clearance; structure footprints occupy whole cell rectangles matching their visible edges. A chunk connectivity graph includes structures and Resource Deposits and guides detailed routes, accepting modest detours to reduce search work while preserving accurate reachability. This replaces ADR-0008's Hauler-only intent-cell graph; Haulers retain the 0.35 owned/visible Corridor cost sampled beneath fine cells, while other types use ordinary distance.

## Consequences

- Completed structures and existing deposit objects block every swarm; all work interactions use reachable exterior positions. Movement segments and local steering must respect body clearance, including corners.
- Shared per-tick search budgets distinguish pending from unreachable. Clearing and invalidated routes receive priority, with aging for routine requests; affected hierarchy regions refresh when blockers change.
- Obstructions invalidate active routes before entry; removing blockers wakes waiting work without requiring valid routes to change. Paint changes affect future legs. Cargo remains physically held during failed or pending routing.
- Nanobots avoid each other locally, including consistent yielding and backing out in narrow passages. Their occupancy never becomes a global route cost or unreachable result; overlapping and teleportation are not escape mechanisms.
- Each Production Facility holds at most one finished nanobot until a free exterior cell exists, pausing further production until that output is released. Blocked exits constrain throughput rather than creating an internal backlog. Attack obstruction is outside this decision.
- A facility waiting for exit space does not satisfy the busy-capacity condition for Production Pressure. Congestion therefore cannot justify automatic facility expansion; adding production capacity would not resolve blocked exits.
- Destroying a Production Facility destroys any finished nanobot retained inside it, without spawning that output or refunding its consumed minerals. Retained output remains exposed to the facility's destruction until it enters the world through a free exit.
- Finished output waiting for an exit counts toward its type's committed population for production planning, preventing duplicate production to cover that commitment. It becomes available for work only after release into the world.
- When facilities compete for the same free exit cell, the longest-waiting finished output has priority; simultaneous completions use a stable tie-breaker. This prevents repeated preference for one facility from starving another.
- Factorio's [published approach](https://www.factorio.com/blog/post/fff-317) abstracts terrain while ignoring entities. Our hierarchy includes structures and deposits because they define the relevant access constraints; chunk sizes and budgets require measured validation.
