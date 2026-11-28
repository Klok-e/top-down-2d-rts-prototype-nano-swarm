---
status: accepted
---

# Rank restart refills by time to fund a production cycle

When demand returns to a Production Facility, it can use up its hopper before the first refill arrives, even with stocked pickups nearby. A cycle is funded once its full mineral cost is in the hopper. This refines production urgency in [ADR-0033](0033-economy-driven-gathering.md).

Forecast when the first cycle that needs a refill is due and when enough minerals will be in the hopper to fund it. Account for the funded cycle in progress, the minerals in the hopper, and when inbound Haulers are expected to unload. Include travel to pickup, loading, travel to the facility, and the time to unload enough minerals. Use the parts of the trip still outstanding for each existing delivery. Track the cycle while demand stays unmet until its minerals are in the hopper. Run the timing forecast once outstanding demand will reach that cycle, including when demand grows during the cycles the hopper can already fund.

When the forecast leaves that cycle waiting, compare the usual pickup with a shorter load that can fund the cycle together with incoming minerals. A candidate earns production urgency when it advances the estimated funding time. Prefer the earliest funding among eligible refills and the usual load on a tie. Score the route against the usual eligible pickup when comparing destinations. The Hauler reserves and carries the selected amount. Charger emergencies and fully aged terminal work follow their existing strict priority rules.

Estimate travel from the remaining movement path when available and from physical routes otherwise. Refine detours within the navigation scheduler’s background allowance. Wait for valid inbound trips to have delivery times before awarding restart urgency or shortening a pickup. Ordinary refills can proceed after a brief wait that starts when an eligible trip first encounters an unresolved route. A refill with a confirmed funding advantage and work in the strict priority tiers remain eligible during the wait.

Refills use the ordinary unreserved delivery window and honor active Logistics Reservations. The forecast uses the tuned hopper capacity and Hauler transfer rate. A full delivery window or a long route can still leave the facility waiting. The intended gain is a shorter restart stall when a faster eligible refill exists.
