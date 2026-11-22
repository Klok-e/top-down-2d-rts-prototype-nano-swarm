---
status: accepted
---

# AI Battle statistics stay in memory

AI Battle keeps its samples, summary, and hitch summaries in memory and writes no result files. Agents read them over the agent socket with cursors for new samples and hitches. Writing and syncing the files on every one-second sample took 8–25 ms on the main thread inside the fixed tick. Data grows by about 1.5 KB per simulated second, about 5.4 MB per hour, so it needs no reduction.

## Considered Options

- **Write files at the end of a run.** Rejected: agents read results over the socket instead of from disk, so files added a second output format.
- **Flush from a panic hook.** Rejected: the hook would need shared state outside the ECS.
- **Simulation-only prints the results.** Rejected in favour of one socket interface for every battle mode.

## Consequences

- A crash or killed process loses the run's statistics.
- Every non-windowed AI Battle, headless or simulation-only, requires the agent socket. After Swarm Elimination or the tick limit, the simulation freezes and the process waits for an explicit shutdown, with no timeout.
- This supersedes the parts of [ADR-0021](0021-ai-battle-benchmark.md) that said each run writes samples and a summary and a headless battle exits after saving.
