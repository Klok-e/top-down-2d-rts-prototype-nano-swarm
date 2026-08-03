# Nano Swarm Handoff

## Objective

Finish the exhaustive headless playtest/fix pass in `/home/dima/Desktop/top-down-2d-rts-prototype-nano-swarm`. Preserve all existing broad uncommitted work, keep every automated/runtime check windowless, and close the final reviewer loop.

Read `AGENTS.md` first. Domain context is in `CONTEXT.md` and `docs/adr/`; headless testing requirements are in `docs/agents/testing.md`. The agent-control protocol is documented in `docs/agents/agent-control.md`.

## Current State

The broad worktree is intentionally dirty and uncommitted. Do not revert or rewrite unrelated changes. `git status --short` currently reports modifications across gameplay, UI, tests, screenshots, and docs, plus new headless control/runtime files. Inspect the actual diff rather than treating any pre-existing changes as disposable.

The latest reviewer session is `ses_086b77e01ffeiTMBNQ8Dq0XMy3`. Resume that same reviewer task for closing confirmation.

## Latest Fix

The reviewer found that Production treats an unowned facility as a player compatibility fallback, while Collapse previously excluded all unowned facilities and could latch Defeat during active production.

Implemented:

- `src/nanobot/production.rs`: made `facility_belongs_to_swarm` `pub(crate)` so Collapse shares Production's ownership rule.
- `src/nanobot/collapse.rs`: facility query now accepts optional ownership.
- Busy or fully funded unowned facilities count as player operational/funded production through the shared fallback.
- Empty unowned facilities remain excluded from repair/supply, local Sink, and Hauler material-path checks, because logistics requires explicit ownership.
- `tests/behavior/production_collapse.rs`: added `busy_unowned_facility_uses_player_fallback_for_collapse_detection` and `unfunded_unowned_facility_is_not_a_player_hauler_destination`.

The positive regression was run before the fix and failed at the collapse assertion, then passed after the fix. All 23 focused collapse tests pass.

## Validation Completed After Latest Fix

- `cargo fmt` passed.
- `cargo clippy --all-targets -- -D warnings` passed.
- `cargo test` passed: 332 unit tests, 428 behavior tests, 19 active playtests; one GPU playtest remains intentionally ignored.
- `cargo test --test screenshots -- --ignored` passed: 17/17. The intentional panic printed by `harness_callback_panic_fails` is expected and the test passes.
- `git diff --check` passed after formatting and the latest fix.

## Runtime Status

A post-fix real process was launched with:

```bash
cargo run -- --headless --agent-socket --width 1280 --height 720
```

`session.hello` and `state.get` succeeded. At fixed tick 656, both swarms were healthy and the match was `in_progress`. Before the explicit shutdown request, the process, runtime directory, and temporary logs had already disappeared; `pgrep` found no remaining game process. Therefore the workspace has no live process, but the post-fix API-shutdown/socket-removal leg was not conclusively observed. A pre-fix final smoke had already verified API shutdown and socket cleanup.

## Remaining Steps

1. Run one short real `--headless --agent-socket` smoke. Check directory/socket modes (`0700`/`0600`), call `session.hello`, `state.get`, optionally wait 60 fixed ticks, call `process.shutdown`, and verify the socket/process disappear. Do not create an OS window.
2. Resume reviewer session `ses_086b77e01ffeiTMBNQ8Dq0XMy3`. Ask it to inspect the latest facility-fallback fix and both regressions, then report findings first. Include the completed validation above.
3. If the reviewer is clean, report completion. If it finds a real defect, reproduce it with a focused regression before changing code, then rerun `cargo fmt`, Clippy, full tests, screenshot tests, `git diff --check`, and reviewer confirmation.
4. Do not commit or push unless explicitly requested.

## Key Commands

The client is `scripts/nano_swarm_control.py`; default socket is `$XDG_RUNTIME_DIR/nano-swarm/control.sock`.

```bash
python3 scripts/nano_swarm_control.py hello
python3 scripts/nano_swarm_control.py state
python3 scripts/nano_swarm_control.py wait --fixed-ticks 60
python3 scripts/nano_swarm_control.py shutdown
```

## Relevant Files

- `src/nanobot/collapse.rs`
- `src/nanobot/production.rs`
- `tests/behavior/production_collapse.rs`
- `src/agent_control.rs`
- `src/runtime.rs`
- `scripts/nano_swarm_control.py`
- `docs/agents/agent-control.md`

## Suggested Skills

- `tdd`: invoke if another reviewer finding requires a code change; preserve the established failing-regression-first workflow.
- `zoom-out`: invoke only if broader production/logistics ownership context is needed before changing contracts beyond this focused fix.
