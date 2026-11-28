---
status: superseded by ADR-0044
---

# Count assigned Defenders at Staging Points

The [Frontline decision](0044-advance-a-bounded-frontline.md) replaces this arrival rule.

The Strategic Controller advances a Staging Point using the fraction of its assigned Defenders that have physically arrived. It observes the destinations chosen by Defender allocation, including assignments whose routes are pending, so the arrival fraction measures the force expected at that point.

An active raid with a reachable approach holds its Staging Point during a brief drop in readiness. It advances when both readiness and arrival are satisfied. A sustained shortfall ends the raid.

This revises the arrival rule in the [fixed Staging Points decision](0030-fixed-staging-points.md).
