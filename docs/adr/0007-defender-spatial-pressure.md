# Defender cell ownership and local pressure

Regional intent allocation from [ADR 0009](./0009-regional-intent-allocation.md) is the sole source of Defender work claims. It projects Defend demand and threat pressure from the Intent Grid, then gives each Defender a regional lease for one supported Defend cell. A supported holder remains attached to that cell while its paint and ownership remain valid; threat pressure attracts idle, new, or replacement Defenders without retargeting a valid active holder.

Defenders may de-clump within their assigned cell through local separation, with containment keeping them inside the cell. Cross-cell movement is allocator-driven. When a holder needs Charge, it may suspend its regional lease for an operational, supplied Charger belonging to the same Defend cell. The charger assignment retains that source-cell identity through travel and charging, and the Defender requests lease resumption after release. A remote, empty, foreign, or degraded Charger is not a valid alternative.

The retired per-Defender global scorer, physical cell-density snapshot, reservation crowding, home-radius exclusion, and retarget hysteresis are not parallel authorities. Shared spatial-pressure code is not required for Defender assignment; local geometry helpers remain separate from allocation and may serve containment or cosmetic spread without acquiring tactical work.

## Consequences

- Regional leases provide one deterministic ownership path for Defend work and replacement capacity.
- Threat pressure changes projected demand without churning supported holders.
- Defend arrival uses an in-cell stop radius instead of exact center-point arrival.
- Separation can de-clump holders locally, but cannot make them drift into another tactical cell.
- Charger sustain is finite, owner-scoped, and local to the held Defend cell; cut-off fronts weaken instead of pulling defenders across the map.
- Idle cosmetic spread must not acquire or mutate `DefendHold` behavior.
