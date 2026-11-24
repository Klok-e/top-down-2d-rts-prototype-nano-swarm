---
status: accepted
---

# Swarm Elimination counts only completed structures

A swarm is eliminated when it has no completed Production Facility, Stockpile, or Charger. Planned Structures still do not count, and surviving Nanobots no longer keep a swarm in the match. This supersedes the survival rule of [ADR-0020](0020-swarm-elimination-match-outcomes.md); its end-of-tick evaluation, Draw on simultaneous elimination, and permanent first result still apply.

Under the old rule, matches could stall indefinitely. In an AI Battle, the losing swarm was left with a few idle Nanobots, no structures, and no way to produce more, while the winner never hunted them down. Every scenario starts each swarm with a completed Production Facility, so the new rule never ends a match before it begins.

## Considered Options

- **Keep Nanobots as survivors and make controllers hunt remnants.** Rejected: the outcome then depends on controller pursuit and reachability instead of a predictable rule.
