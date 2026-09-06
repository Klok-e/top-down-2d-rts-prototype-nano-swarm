# Owner-scoped production capacity commitments

Production capacity grows through stable, owner-scoped commitments rather than Build Zone area or instantaneous production-cycle state. A swarm may hold at most one unfinished Planned Structure for a Production Facility; adding capacity requires 60 consecutive fixed ticks of any typed Population Demand shortage while every operational facility is busy, including periods when no facility is operational. Resolved demand, idle operational capacity, and issuing a facility plan clear accumulated pressure. Completion, destruction, or access cancellation starts a fresh observation window before another plan may emerge. The completed facility chooses what to produce from current demand only after physical funding. This deliberately favors bounded, explainable growth over concurrent demand-sized expansion batches.

[ADR-0017](0017-construction-preserves-friendly-access.md) defines access cancellation and releases the unfinished commitment immediately.

[ADR-0019](0019-demand-driven-automatic-production.md) removes Production Priority while retaining pressure from every typed shortage.
