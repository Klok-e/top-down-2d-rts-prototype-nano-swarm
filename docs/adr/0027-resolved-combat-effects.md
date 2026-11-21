---
status: accepted
---

# Resolved combat effects

Fixed-step combat emits resolved hit and death facts. Damage and destruction complete in the simulation tick that produced the facts. A separate presentation layer consumes those facts in both windowed and offscreen play. Simulation-only applications can omit it.

Each visible hit launches a traveling bolt. The renderer shows the attacker’s muzzle flash and jab at launch and plays impact, flash, and recoil on arrival. The first killing bolt to arrive starts the death effects. A victim without an in-flight killing bolt can be presented immediately. Later impacts use its death position. Nanobot fragments, structure bursts, rings, and decals can outlive the corresponding gameplay entity.

Nanobots remain top-level world-space gameplay entities as specified by [ADR 0004](0004-world-space-nanobots.md). The presentation child draws their interpolated motion, facing and poses without altering their simulation transforms. Swarm colors identify participants. Live tuning controls particle intensity and bloom. Presentation follows the frame clock, and visibility and effect budgets affect only rendering.

Close views show individual shots, impacts, and death effects. Mid-range views simplify shots and add a heat layer from resolved hits. Far views show the contact line, casualty flashes, and passive battle markers at tactical zoom. A quiet fight can leave a temporary ring in the surviving swarm’s color. Camera shake affects only the rendered view, so painting, panning and agent-reported positions continue to use the gameplay camera.
