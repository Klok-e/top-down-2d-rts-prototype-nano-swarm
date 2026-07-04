# Defender spatial pressure

Defenders spread across all visible owned Defend paint by globally scoring cells instead of clustering on a cell center or drifting freely across zone borders. Paint strength raises both attraction and desired occupancy, physical crowding from all nanobots plus defender reservations lowers a cell's score, and future per-cell defend pressure from enemies can raise it; holding defenders retarget only when another cell beats the current cell by a hysteresis margin. This keeps defender spread, threat response, and cosmetic de-clumping tunable from one scoring model while preserving clear tactical ownership of each defender's assigned hold cell.

## Consequences

- Cross-cell spreading is assignment-driven; local drift stays inside the assigned cell.
- Defend arrival can use an in-cell stop radius instead of exact center-point arrival.
- Shared spatial-pressure helpers should be extracted from this work and reused by idle cosmetic spread.
- Idle spread remains cosmetic and must not own `DefendHold` behavior.
