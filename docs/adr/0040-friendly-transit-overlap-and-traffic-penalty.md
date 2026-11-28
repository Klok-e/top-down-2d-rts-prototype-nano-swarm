---
status: accepted
---

# Allow friendly transit overlap with a Traffic Penalty

Friendly Nanobots may overlap during ordinary transit to keep cargo moving through shared passages. A bounded Traffic Penalty encourages new routes to use quieter paths. In the crowded delivery comparison, overlap increased production and the Traffic Penalty changed lane choice with a median production gain of zero.

Ordinary Transit Overlap applies across Nanobot Types within a swarm. A traveller can pass through idle and working friends. Each Work Position admits one Nanobot. An admitted Nanobot continues working as friendly traffic passes. When delivery space opens, the longest-waiting eligible Hauler has priority.

Body spacing applies between swarms and to pairs involving an engaged Defender or a Defender withdrawing under combat control. When engagement begins during an overlap, combat movement restores spacing as space allows. The Defender can fire according to its usual range and cooldown. [Defender engagement](0025-defender-engagement.md)

The Traffic Penalty uses routes currently followed by the traveller's own swarm. It ranks nearby bends on direct routes and alternatives within the region chain chosen by the existing planner. Physical geometry determines reachability.

Count up to about one second of each Nanobot's upcoming travel. For each other Nanobot using a segment, add 5% of that segment's normal travel cost, up to 15%. This adds to the cost after any Logistics Corridor discount. Use one traffic snapshot for each new ordinary travel route after selecting its destination. A change in traffic alone leaves the chosen route and work destination in place.

Production Facilities release finished Nanobots into body-clear exterior positions. A facility with no clear exit holds its paid output until one opens.

Idle spreading is best-effort. Friendly idle Nanobots that stop in overlapping positions can share that space until they move apart.

During Structure Clearing, eligible friendly Nanobots can pass one another while leaving. Evacuation routes use physical geometry. Activation requires an empty footprint and the final friendly access check. [Construction access](0017-construction-preserves-friendly-access.md)

This revises ordinary transit and route scoring in [shared hierarchical navigation](0016-shared-hierarchical-navigation.md).
