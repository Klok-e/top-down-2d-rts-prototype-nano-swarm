# Defender spatial pressure

Defenders spread across all visible owned Defend paint by globally scoring cells instead of clustering on a cell center or drifting freely across zone borders. Binary Defend paint creates one baseline Defender demand per cell, each hostile occupant adds one more, and physical crowding plus defender reservations lowers a cell's score. Threat pressure guides newly available or reassigned Defenders but does not break a supported active lease. This keeps zone area, reinforcement demand, and cosmetic de-clumping legible while preserving stable tactical ownership of each Defender's assigned hold cell.

## Consequences

- Cross-cell spreading is assignment-driven; local drift stays inside the assigned cell.
- Defend arrival can use an in-cell stop radius instead of exact center-point arrival.
- Larger Defend Zones request broader baseline coverage; threats authorize extra Defender production and prioritize reinforcement without churning supported holders.
- Shared spatial-pressure helpers should be extracted from this work and reused by idle cosmetic spread.
- Idle spread remains cosmetic and must not own `DefendHold` behavior.
