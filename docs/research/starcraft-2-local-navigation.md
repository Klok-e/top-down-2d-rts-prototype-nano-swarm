# StarCraft II local navigation and crowded goals

## Blizzard’s published descriptions

James Anhalt separates steering from collision handling in his GDC 2011 talk. Around 10:30 he lists following, flocking, grouping, separation, avoidance, and arrival, crediting Craig Reynolds’ Boids. Around 13:30 he describes pushing and distinguishes moving, stationary, friendly, and enemy units. [GDC recording](https://www.gdcvault.com/play/1014514/AI-Navigation-It-s-Not)

Bob Fitch describes A*, dynamic triangles and zones, circular bodies of different sizes, and group leaders choosing wider routes to keep units together. See slides 34–35 of `Blizzard AI.pptx`. These descriptions leave open how individual workers choose positions at a crowded destination. [AIIDE speaker page](https://movingai.com/aiide11/speakers.html) · [Original slide archive](https://movingai.com/aiide11/BlizzardAI.zip)

## A later implementation inspired by StarCraft II

Aron Granberg credits Anhalt’s talk for an approach to avoiding locked units. For crowded arrivals, he describes estimating density inside a circle centered on the goal, with radius equal to the agent’s remaining distance. Above a threshold the agent can stop. His package uses RVO/ORCA for local avoidance. The density rule and RVO/ORCA attribution describe Granberg’s implementation. [Avoidance API](https://arongranberg.com/astar/documentation/5_2_5_9692d66e7/jobhorizonavoidancephase1.html) · [Crowded arrival explanation](https://forum.arongranberg.com/t/local-avoidance-query/5888) · [Local avoidance documentation](https://arongranberg.com/astar/documentation/stable/localavoidance.html)

Nano Swarm’s choices for exterior work positions, waiting, and congestion recovery are recorded in [ADR 0016](../adr/0016-shared-hierarchical-navigation.md).
