# Fine navigation geometry

The fine grid has 72-world-unit cells, independently of the 512-unit intent paint grid. The shared body radius is 34 units (the existing conservative bound around all three 64-pixel Nanobot silhouettes). A straight one-cell passage therefore leaves two world units per side. Two bodies cannot pass side by side in it.

Support sprites retain their 64-unit local size. `navigation::align_structure` rounds each requested world dimension to the nearest positive whole number of cells and snaps its lower edges to the world-origin grid. A default structure becomes 72×72; the authored 192×192 Production Facility becomes 216×216. Nonuniform 128×192 becomes 144×216. Even cell counts put the center on a cell boundary; odd counts put it at a cell center. Planned and completed forms preserve the same transform. A thin interior outline makes the actual occupied edges visible despite transparent texture margins.

Automatic placement checks the snapped rectangle against the union of eligible owner-scoped paint cells, physical circular deposits, and rectangular structures (including their authored scale and in-tick plans). Its 16-unit obstacle padding is a construction spacing rule, not a promise of a navigable passage. Body clearance is measured from physical shapes; deposits retain their authored circular radius and appearance, including after depletion. No obstacle expands to an intent-cell boundary.

This preparatory geometry change does not install routing, movement collision, clearing, access-preservation checks, or production-exit occupancy. Those consumers can use the shared physical geometry in the subsequent navigation tickets.
