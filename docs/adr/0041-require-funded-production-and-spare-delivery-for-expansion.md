---
status: accepted
---

# Require funded production and spare delivery for expansion

A facility can spend a single delivery and appear busy while its mineral supply is intermittent. A new facility commits Worker time and competes for the same deliveries. Adding capacity requires evidence that the installed facilities are fully used and that a viable site can be supplied.

While Population Demand remains unmet, every operational Production Facility must complete three consecutive funded cycles and start the next cycle on schedule. Waiting for minerals or exit space breaks the streak. Resolved demand clears the funded production streaks. This amends the previous busy-tick rule for expansion. [ADR-0011](0011-owner-scoped-production-capacity-commitments.md)

Use a complete trailing 20-second window to estimate spare delivery. Count recent Worker drop-offs at usable Source Stockpiles and subtract the mineral needs of existing Production Facilities and Chargers. Estimate how much uncommitted Hauler time can deliver on viable direct routes from those Sources to the site. Include the loaded trip, empty return, and time to load and unload at current tuning. Check pickup access and delivery routes with the proposed footprint in place. Both replenishment and hauling must support the full tuned mineral rate of one more facility. Check eligible Build footprints in the usual placement order within the placement search limits. Choose the first site that meets the supply requirement. Evaluate the Haulers available under ordinary economy demand.

A swarm can hold one unfinished Production Facility plan. Issuing a plan clears the funded production streaks, and a valid plan can finish through a later supply dip. After the plan completes, is destroyed, or is cancelled, every operational facility must earn a fresh funded production streak. Recent supply observations remain eligible for the rest of the trailing 20-second window and are assessed against the resulting economy. Unmet demand with zero operational facilities still qualifies for the existing recovery rule. The Strategic Controller provides Build paint for the shared planning system. [Build support policy](0037-retain-build-paint-under-supported-structures.md)
