# Hauler route cost fields

Haulers use route planning for logistics legs, with Logistics Corridor paint acting as a soft route-cost discount rather than a mandatory road or a job source. All hauler source and sink selection should be able to compare route costs, and the chosen leg should move through route waypoints; stronger corridor paint lowers cell traversal cost, while unpainted cells remain valid normal-cost terrain. This replaces ad-hoc single corridor waypoints with a general A* route-cost field that can later accept path blockers without changing the player-facing meaning of corridors.

## Consequences

- Corridors bias both the path taken and logistics travel-cost estimates used in job selection.
- Corridor paint remains hauler-only guidance and never creates hauling jobs by itself.
- Routes are computed per logistics leg and remain stable for that leg; later paint changes affect future legs.
- The initial route graph is the intent-cell grid with 8-neighbor movement, diagonal cost, owned/visible corridor paint, no artificial search bounds, and no congestion/capacity cost.
- Structure/deposit path blocking is separate follow-up work; this decision leaves room for blockers in the route-cost field.
