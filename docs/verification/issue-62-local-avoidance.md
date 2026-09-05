# Local avoidance acceptance

Verified 2026-09-05 for [issue #62](https://github.com/Klok-e/top-down-2d-rts-prototype-nano-swarm/issues/62), against parent #41 and ADR-0016.

The movement implementation landed in `dae230b`, before this acceptance pass. This change closes verification gaps: independent corner/deposit clearance checks, route-retention assertions for opposing streams, and a reproducible real-process bottleneck fixture. No production routing or movement behavior was changed.

## Deterministic coverage

`tests/behavior/local_avoidance.rs` exercises real movement and navigation through the approved shared simulation harness:

- `opposing_types_pass_structure_corners_and_deposits_with_body_clearance`: opposing Workers, Haulers, and Defenders from both swarms pass a rectangular structure and a depleted deposit. Literal rectangle/circle distances and interpolated positions check body clearance throughout each movement step. Both destinations are reached with exactly two completed routes.
- `following_traffic_yields_with_the_front_bot_in_a_bottleneck`: four bodies, three spawn orders, swept separation, and arrival at every original goal. The added assertion requires exactly four completed routes, excluding crowd-driven global detours.
- Existing named cases cover open-space swept passing, one-cell backout with retained routes, trapped safe waiting, wider parallel lanes, pending navigation, and moving destinations.

The new corner/deposit test passed on the existing implementation. Temporarily weakening the swept pair guard from `+ 0.001` to `+ 100.0` made it fail on Worker overlap at tick 100, between `(555,468)/(619,493)` and `(560,468)/(619,498)`. Restoring the guard restored green; the production file has no final diff.

This is verification of existing behavior, not a player-facing production fix. No scripted playtest was changed. The dedicated process fixture below exercises the real runtime plugin stack and external control client in addition to the behavior tests.

## Real headless bottleneck

`examples/local_avoidance.rs` creates the four-body passage in the real offscreen runtime with the agent socket enabled. Two 288 by 72 rectangles leave a 72-unit passage. Each body has 34-unit navigation clearance. The example checks swept pair separation and independent segment-to-rectangle distance after every fixed tick, and requires all four original destinations within 2,400 simulation ticks.

The rectangle-distance checker has six literal unit tests for crossing, stationary inside/outside positions, horizontal/vertical parallel segments, and the nearest corner. A temporary `+ 1.0` distance error made four tests fail; restoring the checker restored all six. These tests run explicitly with `cargo test --example local_avoidance`; evidence is in `target/issue-62/runtime/checker-{red,green}.log`.

The isolated fixture removes the authored economy and its Swarm entities so production-collapse rules do not terminate traffic acceptance. It retains the actual movement, navigation, presentation, and control systems. Distant Corridor paint releases its capture phases; it supplies no route guidance to these Workers. Virtual time pauses at each capture phase while the socket and frame synchronization remain active.

From the repository root, launch in one terminal:

```bash
mkdir -p target/issue-62/runtime/xdg
chmod 700 target/issue-62/runtime/xdg
XDG_RUNTIME_DIR="$PWD/target/issue-62/runtime/xdg" BEVY_ASSET_ROOT="$PWD" CARGO_INCREMENTAL=0 cargo run --example local_avoidance > target/issue-62/runtime/process.log 2>&1
```

In another terminal, prefix each command below with:

```bash
python3 scripts/nano_swarm_control.py --socket "$PWD/target/issue-62/runtime/xdg/nano-swarm/control.sock"
```

1. `hello`, then `wait --frames 2`, then `screenshot --name before`.
2. `paint corridor -3 -3`; use `wait --frames 30` until the process log reports `TRAFFIC retreat`.
3. `wait --frames 2`, then `screenshot --name retreat`.
4. `paint corridor -2 -3`; use `wait --frames 30` until the log reports `TRAFFIC arrived`.
5. `wait --frames 2`, then `screenshot --name arrived`, then `state` and `shutdown`.

The acceptance run used the built example directly with Cargo's dynamic library path. Exact client requests and responses are in `target/issue-62/runtime/commands.json`; phase JSON files contain screenshot paths. The runtime directory was private mode 0700. No OS/compositor window was created.

Both the producing agent and the primary agent inspected the three process captures:

- **Before, tick 0:** four separate green bodies in the gap, with two destination markers on each side.
- **Retreat, tick 23:** the leading right-side body has backed out and moved upward beyond the right wall end; the other three remain separated along the passage.
- **Arrival, tick 244:** two bodies stand at each side's destination markers and the passage is empty. Logged final coordinates are `(828,324)`, `(900,324)`, `(180,324)`, and `(108,324)`.

Swept assertions passed every fixed tick. Client shutdown succeeded, the process exited 0, the socket disappeared, and the advisory lock was released. The final process log is `target/issue-62/runtime/process.log`. Earlier diagnostic captures from fixture setup are excluded from this evidence.

## Repository checks

The main Cargo check/test gates used `CARGO_INCREMENTAL=0`. Logs are under `target/issue-62/`.

| Check | Result |
| --- | --- |
| `cargo fmt` | Passed |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo test --test behavior local_avoidance` | 8 passed |
| `cargo test` | 363 unit, 513 behavior, 45 playtests passed; 2 GPU playtests and 37 screenshot trials ignored |
| `cargo test --test screenshots -- --ignored` | 37 passed |
| `cargo build --example local_avoidance` and example Clippy | Passed |
| `cargo test --example local_avoidance` | 6 passed |
| Standards / Spec review | 0 remaining findings on either axis |

The fresh offscreen run produced 71 PNGs, listed in `target/issue-62/fresh-captures.txt`. All were inspected through six contact sheets; `local_avoidance_{before,yield,arrived}.png` were also opened individually and show separated passage occupancy, local backout, and completed crossing. These captures agree with the independently inspected process frames.

Standards review identified the missing oracle tests; those were added and mutation-checked. Both reviewers also recommended named capture phases, now represented by an enum. Focused re-review passed after the changes, and the Spec reviewer independently inspected all three replacement process captures from the final replay.

The evidence establishes the representative traffic cases required by #62. It does not establish progress in physically impossible layouts or the separate 5,000-nanobot performance acceptance in #66.
