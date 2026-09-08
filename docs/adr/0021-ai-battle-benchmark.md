---
status: accepted
---

# AI Battle measures shared autonomy until Swarm Elimination

AI Battle uses two copies of the existing timed intent controller, equal starting setups, and the Standard map to exercise production, logistics, navigation, and combat together. This establishes a repeatable workload for performance and nanobot behavior comparisons; developing adaptive strategic AI is outside this decision. The scenario is selectable for watching in the game with painting disabled and can also run headlessly.

Scenario definitions own authored setup and select session policies for player control, outcome presentation, statistics collection, and headless pacing. Gameplay, input, UI, and recording consume those policies without branching on scenario identities; adding a scenario should not require special cases throughout those systems.

Both modes use the same fixed simulation timestep and starting seed. The watchable scenario runs at normal speed; headless runs as fast as possible. These shared inputs improve repeatability without promising identical outcomes from a seed alone.

Extend the Swarm Elimination semantics in [ADR-0020](0020-swarm-elimination-match-outcomes.md) to AI Battle: evaluate both sides together after a simulation tick's entity creation and removal, identify the surviving swarm as winner, and record Draw for simultaneous elimination. The first outcome is permanent. There is no time limit or automatic stalemate cutoff, so an unresolved battle can run indefinitely.

At battle end, record the outcome and elapsed simulation time, flush results, and stop collecting statistics. The watchable simulation continues until manually stopped; headless exits after saving. This collection boundary keeps post-battle inactivity from diluting battle measurements. Ctrl+C saves collected data and marks an unfinished battle as interrupted without inventing a winner or Draw; an already recorded outcome is preserved.

Collect simulation tick timing, frame timing for watchable runs, and population counts to relate performance to battle size. Record population by Nanobot Type, births and deaths, resources gathered and consumed, structures built and lost, and elimination time per swarm. Sample once per simulated second and retain tick-time percentiles so sampling does not conceal performance spikes.

Each run writes CSV time-series data and a summary in its own directory. Record scenario settings, including the timestep and seed, code revision, and the first elimination outcome when one occurs. Flush periodically so manually stopped runs retain collected data. The saved results support comparisons across code revisions; CSV and summary details remain implementation choices rather than a promise of exact replay.
