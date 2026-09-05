# Congested work and traffic recovery

Verified 2026-09-05 against the accepted local movement rules in [ADR-0016](../adr/0016-shared-hierarchical-navigation.md). This implements the project decisions; it is not a claim to reproduce Blizzard's private algorithm. Source research is in [StarCraft II local navigation](../research/starcraft-2-local-navigation.md).

## Reproduced failures

Three working friendly bodies occupied the west approach of a terminal while other faces remained free. The loaded Hauler retained 20 minerals and delivered nothing after 60 simulation seconds. An equivalent planned-construction fixture retained all five units of required work. A sealed single-file passage also left two opposing friendly travellers permanently stopped.

The fixes address several interacting causes:

- Static reachability alone selected an occupied work approach. Local destination selection now searches and claims separate, reachable perimeter positions, preserving pending candidate identity while routing completes.
- Sparse perimeter candidates could miss an actual free face-center position after a waiter moved aside. Dense work candidates are separate from the unchanged static navigation candidates.
- Repulsion could exactly cancel the final movement toward a waypoint during recovery. Travelling recovery uses route steering; queued or route-less recovery retains outward separation so it cannot remain embedded in a working body.
- Progress is measured along the retained route, so normal detours do not accumulate false stall time. Friendly overlap remains temporary; static geometry and hostile body clearance still constrain every movement segment.

Work effects require separate standing space. Invalid-task cleanup remains available while work is blocked. Empty bots can choose alternative useful work; loaded queues retain cargo and reservations, and oldest eligible arrivals retain priority when yielding. Rejected-goal filtering stays inside the allocator's existing candidate budgets.

## Deterministic checks

The new playtests are registered in `tests/playtest.rs`:

- `congested_economy::hauler_repeatedly_delivers_around_occupied_terminal_approach` checks two real deliveries, cargo conservation, working-body immobility, and separate unloading space at both 100 ms and 60 Hz cadence.
- `congested_economy::worker_builds_around_occupied_site_approach` checks actual construction through other free faces at both cadences.
- `congested_economy::hauler_sustains_twenty_deliveries_around_occupied_goal_at_runtime_cadence` checks 20 autonomous trips and 400 minerals delivered at 60 Hz, including rolling progress deadlines and conservation on every tick.

`tests/behavior/congestion.rs` covers passage recovery, wall clearance, bounded displacement, restoration of spacing, immediate idle yielding, detour progress, and a destination becoming queued during overlap. `tests/behavior/work_navigation.rs` covers oldest-waiter priority, empty-task release, and actually building at an alternative site.

Existing collision fixtures distinguish hostile blocking from accepted friendly recovery. Simultaneous-unload fixtures use separate working positions. Arrival tests allow completed idle bots to yield to travellers still passing. These changes preserve physical-clearance and task-completion assertions under the accepted rules.

`cargo test` passed: 359 unit tests, 529 behavior tests, and 50 playtests; two GPU playtests remain ignored by that command. Formatting and `cargo clippy --all-targets -- -D warnings` passed. Logs are under `target/congestion/`.

Mutation verification reintroduced Euclidean-only progress, disabled proactive idle yielding, and suppressed separation during queued recovery. The three corresponding behavior tests failed. Restoring the implementation restored all four congestion tests to green, and Clippy passed again. See `regression-mutation.log` and `regression-restored.log` in the evidence directory. Code review found no remaining issues after the recovery and bounded-allocation corrections.

## Inspected rendered evidence

The full-app offscreen `congested_work` trial asserts and captures an occupied west approach followed by physical delivery of all 20 minerals from the south face. Both PNGs in `target/playtest-screenshots/congested_work_*.png` were independently inspected: the three working bodies retain their positions, the Hauler occupies separate space south of the sink, its cargo bar disappears, and the sink's resource bar appears.

All 38 offscreen trials passed with `cargo test --test screenshots -- --ignored`. The command used a process-local `ulimit -n 8192` after the default 1024-descriptor limit exhausted audio/backend handles during the suite. All 73 resulting PNGs were inspected, using contact sheets for the existing scenes and individual images for ambiguous details and the new congestion scene. No blank or garbled frames or unexpected working stacks were found. The collision screenshot now permits friendly overlap only during recovery and allows arrived idle bots to yield to traffic.

## Real-process run

The normal binary ran with `--headless --agent-socket --width 1280 --height 720`, using a private mode-0700 runtime directory. Every action, state read, wait, capture, and shutdown went through `scripts/nano_swarm_control.py`. The driver retained at `target/congestion/runtime_driver.py` derives from `scripts/navigation_runtime_acceptance.py`, selects only normal gameplay, and takes six samples separated by 1,200 fixed ticks after the initial capture.

All seven PNGs under `target/congestion/runtime/normal/` were inspected. Names are capture labels; adjacent samples are approximately 20 simulation seconds apart.

| Capture | Fixed tick | Player minerals | Workers / Haulers / Defenders | Facilities |
| --- | ---: | ---: | --- | ---: |
| start | 39 | 0 | 4 / 2 / 3 | 1 |
| working | 1263 | 208 | 4 / 2 / 3 | 2 |
| delivery | 2485 | 148 | 4 / 6 / 3 | 4 |
| later | 3705 | 192 | 4 / 7 / 2 | 4 |
| minute_one | 4947 | 154 | 5 / 5 / 3 | 4 |
| minute_two | 6176 | 78 | 5 / 5 / 3 | 4 |
| final | 7400 | 110 | 5 / 6 / 2 | 3 |

The run spans approximately 123 simulation seconds. New support structures and production facilities appear, mineral custody changes, and production continues. Combat affects later health and population. Neither swarm reports collapse at the final sample. The process exits zero after client shutdown, removes its socket, and releases its advisory lock; `cleanup.json` records these checks.

This is representative live integration evidence, not proof that every bot progresses in every map. Still images cannot distinguish every individual pause from congestion, demand, or combat; the deterministic sustained-delivery test supplies the repeated-transport oracle. A new 5,000-bot benchmark was not run.
