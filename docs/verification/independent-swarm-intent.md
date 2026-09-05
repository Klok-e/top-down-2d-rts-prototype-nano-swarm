# Independent swarm intent verification

ADR-0018 is implemented across paint storage, simulation, mouse controls, agent state, and the zone overlay. Every paint edit requires a swarm; contests, capture timers, and unowned paint APIs are removed.

Validation on 2026-09-05:

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `git diff --check` passed.
- `cargo test`: 359 unit, 522 behavior, and 47 scripted playtests passed. Two existing ignored playtests remain ignored.
- `cargo test --test screenshots -- --ignored`: all 37 offscreen checks passed. All 71 fresh PNG artifacts were inspected, with 13 relevant frames reopened at full resolution; no visible failures were found.
- The ownership regression failed when a temporary mutation blocked second-swarm paint; the mutation was restored before the passing full run. The collapse regression failed before removing whole-cell occupancy exclusion, then all 24 collapse tests passed.
- Read-only implementation review found the collapse exclusion; its fix and regression received a clean follow-up review.

Deterministic coverage includes three-swarm overlap for all four kinds, selective erasure, dirty-state propagation, shared finite-deposit extraction, mutual Threats and population demand, Charger eligibility under overlap, and separate owned construction in the same cell. Production and collapse recovery use physical footprints rather than excluding an entire occupied Build cell.

The scripted input flow `mouse_zone_painting::scripted_player_paints_and_erases_independent_overlap_for_every_kind` exercises keyboard selection and left/right mouse state for all four kinds, including overlap-to-enemy-only rendering after erasure.

A real `--headless --agent-socket --width 1280 --height 720` process was controlled only through `scripts/nano_swarm_control.py`. Gather at `(4, 0)`, Build at `(3, -1)`, and Defend at `(2, 0)` reported owners `[0, 1]` after painting and `[1]` after player erasure. State omitted `defend_contests`. Both captured frames were inspected: overlap preserves player intent colors beneath enemy hatching; erasure leaves enemy paint and structures visible.

Local artifacts:

- `target/independent-intent-proof/independent_overlap.png` (capture tick 39)
- `target/independent-intent-proof/enemy_orders_preserved.png` (capture tick 73)
- `target/independent-intent-proof/transcript.json`
- `target/playtest-screenshots/zone_binary_overlay.png`

The process shut down with exit 0; its socket disappeared, its lifecycle lock was available, and temporary runtime directories were removed. All visual verification used offscreen rendering without an OS/compositor window.
