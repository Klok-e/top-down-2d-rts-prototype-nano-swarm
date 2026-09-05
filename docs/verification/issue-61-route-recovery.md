# Issue #61: useful work and cargo recovery

Verified 2026-09-05 against starting commit `1ebbcf1`. Scope is issue #61 under #41; the 5,000-Nanobot aggregate benchmark remains #66.

## Behavior

Task acquisition selects proven reachable work without letting another pending candidate suppress it. Empty commitments release assignments and source/destination reservations when their route becomes unreachable. Shared `WorkAccess` keeps Pending, Reachable, and Unreachable distinct for Population Demand and Production Collapse. Recovery checks follow a pickup route's actual exterior endpoint into delivery, rather than combining disconnected faces of an object.

Workers and Haulers revalidate loaded destinations, transfer reservations to reachable alternatives, and keep physical cargo while no destination is available. Haulers preserve Source-to-Sink and Sink-to-terminal priorities before returning to a compatible Stockpile. Interrupted partial pickup/extraction abandons only the uncollected remainder. Navigation delay preserves the existing commitment; obstacle removal makes waiting work eligible again.

## Deterministic coverage

New tests use the approved simulation and Navigation seams, literal material balances, and explicit bounded updates:

- `tests/behavior/task_reachability.rs`: pending demand, proven inaccessible demand, reopening; pending versus disconnected recovery; a reachable haul leg alongside another pending candidate; inaccessible empty pickup reservation release.
- `tests/behavior/worker_route_recovery.rs`: inaccessible delivery waiting/reopening, reachable replacement, pending commitment, blocked extraction destination, and interrupted partial extraction.
- `tests/playtest/physical_logistics_flow.rs`: `blocked_delivery_returns_cargo_to_source_stockpile_physically`, `blocked_delivery_waits_with_cargo_then_reopens_without_duplicate_transfer`, `blocked_delivery_prefers_another_sink_before_returning_to_source_for_both_swarms`, `interrupted_partial_pickup_delivers_only_the_cargo_already_loaded`, and `terminal_delivery_tries_another_terminal_before_returning_to_pickup_sink`.

Red-before-green runs reproduced blocked returns, retained invalid claims, stranded partial loads, suppressed reachable work, and premature terminal fallback. A temporary reversed fallback-priority mutation also failed the alternate-destination test, then was restored. Existing fixtures were updated where they assumed immediate search completion or placed bodies inside obstacles; their original ownership/capacity assertions remain.

## Offscreen and runtime evidence

All 37 offscreen trials passed with `cargo test --test screenshots -- --ignored`. The new `route_recovery` full-app scenario waits for the actual unreachable result before capture. Its initial frame-20 assumption failed during the complete suite and was replaced with bounded outcome synchronization.

Inspected `target/playtest-screenshots/route_recovery_waiting.png`: a loaded Hauler stands left of the full-height brown wall; the blue destination is on the opposite side. Assertions establish 12 carried minerals, zero stored minerals, and released destination capacity. Inspected `route_recovery_delivered.png`: the wall is absent and the Hauler stands at the blue Stockpile's exterior, with all 12 minerals transferred. Readback runs the real app schedules; the callback rechecks custody when it resumes.

A fresh process used `cargo run -- --headless --agent-socket --width 1280 --height 720`, private mode-0700 `XDG_RUNTIME_DIR=/tmp/nano61.t0Vyxn`, and `scripts/nano_swarm_control.py`. The scripted commands were `hello`, `camera 180 250 1.5`, `state`, `screenshot`, `paint corridor -1 0`, `paint corridor 0 0`, `wait --fixed-ticks 900`, `state`, `screenshot`, and `shutdown`.

The run advanced from fixed tick 7 to 928 before the final capture. State reported player W4/H5/D3, 227 minerals, two facilities with one producing, and an in-progress match. The inspected final image shows Workers outside the deposit, Haulers between Source/Sink/production, completed Stockpiles, and ongoing production; its later capture shows 228 minerals. This verifies ordinary physical-logistics app wiring; forced blockade/reopening is proved separately by the deterministic and full-app offscreen scenarios. Shutdown at tick 938 exited successfully, removed the socket, and released its lock.

An earlier short run ended before extraction began; a longer unattended preliminary run continued into combat and defeat. Neither is used as the final logistics acceptance run. Initial launch attempts exposed the Unix socket path-length limit and missing dynamic-library paths when bypassing Cargo; the final run used the short runtime path and Cargo launcher.

## Checks and review

- `cargo fmt` and `git diff --check`: pass.
- `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: 363 unit tests, 505 behavior tests, and 45 playtests pass. Two GPU playtests and the 37 screenshot trials are ignored by default; the screenshot suite ran separately as above.
- Independent Standards review: no documented violations. Shared reachability was moved into its own module following the review; explicit origin-specific lease mapping remains a non-blocking taste observation.
- Independent Spec review: the Sink-origin terminal-priority defect was reproduced and fixed with the named regression; follow-up review reports no findings.

Logs, the runtime driver, client responses, mutation evidence, and copied runtime captures are under `target/issue-61-verification/` (gitignored). Scale, queue-latency tuning, and per-facility reconstruction of the existing aggregate recovery-material accounting are not claimed by this verification.
