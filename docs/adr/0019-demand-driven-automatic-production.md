---
status: accepted
---

# Demand-driven automatic production

Production Priority is removed as both a player control and an authored opponent policy, superseding ADR-0013's weighted shortage ordering because that indirect control duplicates Population Demand without creating distinct work. Every explicitly owned Production Facility serves its swarm's greatest relative typed shortage, breaking equal ratios by the larger missing count and then stable type order; funded cycles retain their type, and facilities idle once demand is covered. Population Demand becomes required, hidden weights and compatibility fallbacks are removed, and the agent protocol advances to version 2 rather than preserving the deleted interface.
