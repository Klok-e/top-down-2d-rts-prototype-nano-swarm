# Hauler route cost fields

Haulers use route planning for logistics legs, with binary Logistics Corridor paint acting as a soft route-cost discount rather than a mandatory road or a job source. All hauler source and sink selection should be able to compare route costs, and the chosen leg should move through route waypoints; an owned painted corridor cell has 0.35 times normal traversal cost, while unpainted and enemy corridor cells remain valid normal-cost terrain. ADR-0016 supplies shared hierarchical navigation and physical blockers while preserving this player-facing meaning of corridors.

## Consequences

- Corridors bias both the path taken and logistics travel-cost estimates used in job selection.
- Corridor paint remains hauler-only guidance and never creates hauling jobs by itself.
- Routes remain stable for a logistics leg across paint changes; physical obstructions invalidate them before blocked space is entered.
- [ADR-0016](0016-shared-hierarchical-navigation.md) replaces the intent-cell route graph with shared fine-grid hierarchical navigation. Fine cells sample owned/visible Corridor paint; nanobot congestion remains local avoidance rather than traversal cost.
