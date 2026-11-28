---
status: accepted
---

# Advance a bounded Frontline with Charger support

The Strategic Controller adds adjacent reachable Defend paint as charged Defenders reach the leading edge. It advances when enough of that force can fight and recharge to face the local opposition. Ordinary retargeting keeps the existing Frontline and plans the next step from it. Sustained mineral delivery and usable Charger slots set the width with an allowance for existing Swarm Tiles. A slot can support the Frontline when a Defender can reach it, wait for admission, and receive its first recharge pulse within its remaining Charge time.

The controller holds the Frontline through brief support losses. A sustained shortfall with no credible recovery prompts a gradual withdrawal toward reachable support. As it retires rear Defend paint, it retains a reachable safe fallback where one exists and the entire Building Footprint of each Charger it intends to build or use. A Charger the swarm still relies on keeps its footprint until a stocked replacement can take over and active rotations finish.

A credible attack on an Owned Resource Deposit pauses the Frontline's advance. The controller temporarily reinforces the threatened deposit within its paint budget while the Frontline and its Charger support stay painted.

Charger planning forecasts low Charge for every swarm using a shared lead time for construction and mineral delivery. The planner ranks a bounded set of buildable, supplyable sites by how many Defenders can recharge there in time and how soon minerals can arrive. An unfinished or empty Charger reserves future slots while it has a plausible path to construction and supply within a progress grace window. A stalled commitment can earn one replacement for the same demand when a viable site offers better service. Defenders enter rotation at the configured low-Charge threshold.

This decision replaces the advance rules in [ADR 0030](0030-fixed-staging-points.md) and [ADR 0039](0039-count-assigned-defenders-at-staging-points.md) and revises the Charger construction trigger and site selection in [ADR 0025](0025-defender-engagement.md).

## Verification

Verify that the controller finishes an exposed target in a scripted case. Compare Charge-expiry deaths, Charger availability, and Defend erasures over equal simulated intervals in matched Standard and Flanks self-play with pinned balance settings.
