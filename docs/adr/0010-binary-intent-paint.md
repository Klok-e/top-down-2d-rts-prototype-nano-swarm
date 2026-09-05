# Binary intent paint

Intent paint is binary per swarm, kind, and location under [ADR-0018](0018-independent-swarm-intent.md): painting sets intent, repeated painting has no further effect, and erasing clears only the acting swarm's selected kind immediately. Paint intensity and player-set task priority are removed because their extra control made play too complex and confusing; zone geometry, useful work, distance, type fit, crowding, commitments, and threats now drive allocation, while owned Logistics Corridors apply one fixed route-cost discount.
