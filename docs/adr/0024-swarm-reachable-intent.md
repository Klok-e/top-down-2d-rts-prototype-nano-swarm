# Swarm-reachable intent

Intent is limited to the permanent-terrain region a nanobot can reach from its owning swarm's home, assessed at each intent-cell center using normal navigation clearance. The scenario derives and reuses this map for every swarm: brushes skip invalid cells, programmatic paint is rejected, strategic plans remove invalid owned paint on review, and a blocked home rejects the scenario because grid bounds alone include inaccessible exterior space.
