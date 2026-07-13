# Defender spatial pressure

Defenders spread across all visible owned Defend paint by globally scoring cells instead of clustering on a cell center or drifting freely across zone borders. Binary Defend paint gives each cell one Soft Work Slot, physical crowding from all nanobots plus defender reservations lowers a cell's score, and per-cell defend pressure from enemies can raise it; holding defenders retarget only when another cell beats the current cell by a hysteresis margin. This keeps zone area, threat response, and cosmetic de-clumping legible while preserving clear tactical ownership of each defender's assigned hold cell.

## Consequences

- Cross-cell spreading is assignment-driven; local drift stays inside the assigned cell.
- Defend arrival can use an in-cell stop radius instead of exact center-point arrival.
- Larger Defend Zones request broader baseline coverage; threats can pull extra defenders despite each cell's single preferred slot.
- Shared spatial-pressure helpers should be extracted from this work and reused by idle cosmetic spread.
- Idle spread remains cosmetic and must not own `DefendHold` behavior.
