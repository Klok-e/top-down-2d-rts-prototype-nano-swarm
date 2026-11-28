# Papercuts and Maintenance Suggestions

- Fixed-tick economy fixtures use `Tuning::default()`, whose material transfer interval is 75 ticks. The tracked `config/balance.ron` uses 24 ticks. Add a shared test helper for timing measured in material pulses so economy playtests expose the chosen interval and remain readable when tuning changes.
- Offscreen logistics fixtures can silently suspend Sources and Production Facilities if they omit owned Gather or Build paint under the structure footprints. Share a fixture helper that paints each footprint and verifies that the structure is operational before waiting for deliveries.
- Two ADRs are numbered 0033, covering economy-driven gathering and Swarm Elimination. Give Swarm Elimination an unused ADR number and update its references so each decision has a unique identifier.
