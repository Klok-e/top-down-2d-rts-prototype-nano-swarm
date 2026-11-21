---
status: accepted
---

# Defender engagement

Every living Defender can fire within attack range, including during Charger travel and service. In range it selects the nearest hostile Defender, then the nearest other living hostile nanobot, then the nearest operational hostile structure. Stable simulation identity breaks distance ties. Nanobots and structures receive these identities independently of entities created for rendering. A response directs pursuit independently of firing. This lets a staged Defender protect nearby workers and lets several Defenders fight along the same front.

The first shot on acquiring an in-range target is delayed by a stable per-Defender offset from zero through one tick short of the attack interval. Each subsequent hit schedules the next at the attack interval plus deterministic jitter. Switching targets while a hostile stays in range preserves cadence. After an empty tick, the next shot waits for both acquisition and any remaining recovery. The default interval is 40 ticks with ±10% jitter. The Defender's identity and the fixed tick of the hit determine the variation.

Defenders fire within 120 world units by default. A full-strength hit deals 13 damage to a full-strength Defender, 26 to a Worker or Hauler, and 7 to a structure. Two full-charge Defenders take about five seconds to resolve a duel. The 0.25 structure damage factor keeps a lone Defender from destroying a structure as quickly as a nanobot.

## Threat response

Every hostile on a Swarm Tile is a Threat. A hostile Defender accepts two responders by default and every other Threat accepts one. Within regional candidate bounds, each free assignment goes to the best remaining Defender–Threat pair with spare capacity. Pairs rank by danger tier (Defender, other nanobot, structure), regional distance, physical distance, Defender identity, then Threat identity. Existing valid claims persist. Only higher-tier Threats can preempt them. A claimed target may leave territory into the one-cell Pursuit Halo, including diagonal neighbors. Defenders can cross any traversable cells in pursuit. Changes in Threats and Defend paint affect destinations on the next fixed step.

An engaged Defender has a hostile Defender within twice its attack range and is outside Charger travel and service. It approaches or withdraws from the nearest hostile Defender at normal speed to reach a band from 32 to 4 world units inside attack range. Inside that band it strafes at 35% speed. A deterministic per-Defender period of 60–120 ticks governs changes in strafe direction. Separation scales with overlap depth. Forward pressure at the front redirects sideways toward the less crowded friendly flank. Normal speed limits apply to the combined movement.

## Staging and Population Demand

Staging distributes available Defenders equally per Defend cell. Layouts first minimize incumbent relocation, then current-cell relocation. Small cohorts minimize total travel exactly after these continuity constraints. Large cohorts use deterministic bounded-nearest matching to keep paint edits responsive. Cross-cell redistribution uses normal travel speed. Density-driven roaming continues between cells and through deterministic subcell waypoints. Without Defend paint, the swarm stages on its Swarm Tiles. Without any Swarm Tiles, Defenders roam their current cells.

Defender Population Demand is the greater of half the swarm's unique Swarm Tile count rounded up and its active Threat count. Multiple layers in a cell count once per swarm. Response capacity changes the distribution of existing Defenders. Defend paint positions that population and determines where Chargers may operate. All paint retains independent swarm ownership under [ADR 0018](0018-independent-swarm-intent.md).

## Charge and support

Empty Charge gives zero attack and defense. Positive Charge below the weakened threshold scales linearly from the configured strength floor (0.4 by default) to full strength at the threshold.

Low-charge Defenders select the nearest valid supplied Charger in owned Defend paint. Each Charger admits up to three Defenders, within a swarm-wide rotation cap of half the living Defender population rounded down, with a minimum of one. Scarce capacity goes to the lowest Charge, then to staged Defenders, then by stable identity. Admission releases the current response. Recharged Defenders re-enter current response or staging allocation.

Admission covers travel and charging. Casualties can reduce the cap below the number already admitted. Those Defenders finish, and new admissions wait until the active count falls below the cap. An invalid or destroyed Charger ends its rotations. Selecting a replacement requires fresh admission.

Charger construction responds to unserved low Charge by choosing the nearest eligible non-overlapping site in owned Defend paint. A completed Charger outside that paint remains standing and inactive. A valid Charger creates Maintenance demand while serving a Defender or while a living same-swarm Defender is in its cell or a neighboring cell, including diagonals. Unattended Chargers may decay.

## Disengagement and rout

An engaged Defender below the weakened Charge threshold disengages and releases its response. It withdraws until all hostile Defenders are at least the attack range plus two body diameters away. It holds there while requesting normal Charger admission. Disengagement lasts until Charge recovers to the low threshold.

An engaged Defender compares combined local strength within the rout radius. Each living Defender contributes its health fraction multiplied by its Charge strength. Routing friends are excluded, while hostile strength includes all living hostile Defenders. Below the configured ratio for consecutive confirmation ticks, the Defender releases its response and retreats at normal speed to the nearest safe Defend cell. With no Defend paint it uses the nearest safe Swarm Tile. A safe cell has no hostile Defender within the rout radius of its center. Ties prefer the lower row, then column. Without a safe cell the Defender stays in the fight.

A rout ends after the minimum duration once every hostile Defender is more than twice the rout radius away. A Defender that is still disengaged continues to request Charger admission. Other Defenders return to response and staging allocation. If the retreat cell center is obstructed, the Defender navigates to a reachable point in the same cell. Withdrawal overrides engagement steering. In-range firing continues during retreat.
