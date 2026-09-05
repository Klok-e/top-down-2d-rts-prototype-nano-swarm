# StarCraft II local navigation and crowded goals

Research date: 2026-09-05. Scope: design evidence for moving around occupied work positions; no gameplay implementation or freeze reproduction.

## Verified Blizzard evidence

James Anhalt's StarCraft II portion of the GDC 2011 session separates steering and collision handling. The steering slide lists following, flocking, grouping, separation, avoidance, and arrival, and credits Craig Reynolds' Boids. Its collision slide allows units to push other units and distinguishes friendly/enemy and moving/stationary cases. These slides were visually inspected in frames extracted from the public recording at approximately 10:30 and exactly 13:30. [GDC recording](https://www.gdcvault.com/play/1014514/AI-Navigation-It-s-Not)

Bob Fitch's AIIDE 2011 presentation describes the fourth Blizzard RTS pathing generation with A*, dynamic triangles and zones, variable unit sizes, and circular unit bodies. It also describes group leaders and wider routes around obstacles to keep groups together. This concerns group routing; it does not establish a crowded-worksite slot-selection algorithm. Evidence: `Blizzard AI.pptx`, slides 34–35, XML text directly inspected. [Conference speaker page](https://movingai.com/aiide11/speakers.html), [original slide archive](https://movingai.com/aiide11/BlizzardAI.zip)

The GDC slide-download archive returned HTTP 403 during this investigation. The public video stream was accessible, so the two slide findings above are verified from frames rather than inferred from the session listing. The full spoken explanation and Q&A were not transcribed. No verified claim is made here about Blizzard's exact gap-selection rule, unique destination allocation, recovery timeout, or worker collision exceptions.

## Explicitly inspired implementation, not Blizzard's code

Aron Granberg documents `JobHorizonAvoidancePhase1` as inspired by StarCraft II's avoidance of locked units and links Anhalt's talk. This verifies the author's stated inspiration, not algorithmic identity with Blizzard's implementation. [Author's API documentation](https://arongranberg.com/astar/documentation/5_2_5_9692d66e7/jobhorizonavoidancephase1.html)

Responding to a request for swordsmen to surround a target instead of getting stuck behind each other, Granberg describes an improved beta with an SC2-inspired algorithm. He separately explains an approximate crowded-destination stopping rule: measure unit density inside a circle centered on the destination whose radius is the agent's remaining distance. Stop when density exceeds a threshold. He explicitly distinguishes local avoidance from pathfinding reachability. This is evidence for his implementation, not a Blizzard rule. [Author's explanation, November 2018](https://forum.arongranberg.com/t/local-avoidance-query/5888)

Granberg's local-avoidance documentation identifies its implementation as RVO/ORCA. That attribution should not be transferred to StarCraft II merely because another part of the package cites SC2 as inspiration. [Author's local-avoidance documentation](https://arongranberg.com/astar/documentation/stable/localavoidance.html)

## Accepted interpretation for Nano Swarm

The following is an accepted project decision, not a claim about StarCraft II. See [ADR-0016](../adr/0016-shared-hierarchical-navigation.md).

- Treat circling as purposeful movement around occupied approaches to a reachable free work position on another side of the goal.
- Try those positions before declaring the destination full and applying the accepted loaded-wait / empty-retask policy.
- Stop at a valid separate work position. If none is available, wait clear of traffic or reconsider work; continuous orbiting should not count as productive progress.
- Keep this separate from congestion recovery. Seeking another side should happen during ordinary approach, while the accepted temporary-overlap recovery addresses stalled transit.

The public sources support treating steering, collision response, and crowded arrival as distinct concerns. They do not establish that any one algorithm guarantees the reported delivery/building freeze cannot happen. The freeze still needs reproduction and diagnosis.
