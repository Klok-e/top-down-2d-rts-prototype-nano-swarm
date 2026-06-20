# Testing

This document defines repo-specific testing conventions. For the TDD loop itself, use the TDD skill.

## Commands

Run one focused test while developing:

```bash
cargo test <test_name>
```

Run one integration test file:

```bash
cargo test --test <file_name_without_rs>
```

Before handoff, run:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

`cargo test` runs unit tests and automated integration tests. Ignored checks run only when requested:

```bash
cargo test -- --ignored
```

## File layout

```txt
src/
  lib.rs
  main.rs
  ...modules with #[cfg(test)] unit tests...

tests/
  common/
    mod.rs
  *_behavior.rs
  *_visual.rs
```

`tests/common/mod.rs` is the shared test seam introduced for
issue #17. Every `*_behavior.rs` file pulls in the helpers via
`mod common;` and uses the `sim_app_with_*` builders and
`spawn_*` helpers from there. New behaviour tests should follow
the same pattern:

1. `use ... mod common;` at the top of the test file.
2. Replace any local `build_app`, `cell_world_center`,
   `spawn_*` helpers with the canonical ones in `common::`.
3. Pick the smallest `common::sim_app_with_*` builder that
   covers the test's plugin set (e.g. `sim_app_with_build` for
   a build-zone contract).
4. Specialised spawn helpers that need a marker or a non-default
   field (e.g. an `OwnerSwarm`, a `Health::default()` set to
   non-full) belong in `tests/common/mod.rs` next to the rest
   of the seams, not duplicated in the test file.

Unit tests live beside the module they test. Automated integration tests live in `tests/*_behavior.rs`. Temporary screenshot checks live in `tests/*_visual.rs` and must be ignored.

## Unit tests

Use unit tests for behavior that belongs to one Rust module and can be checked without a Bevy `App`, ECS schedule, spawned world, or public crate boundary. Put them beside code they exercise.

Good unit-test targets in this repo:

- Local data encoding, masking, bounds, and validation rules.
- Module-owned indexing helpers and boundary checks.
- Pure math helpers such as coordinate conversion, distance checks, scoring, sorting, and clamping.
- Small local state transitions that only need normal Rust structs/enums.
- Extracted helper functions from systems when system logic is mostly calculation.

Prefer extracting pure functions over mocking Bevy queries, resources, commands, assets, or windows. Use fakes only at real module boundaries, not to simulate Bevy internals.

If test needs `App::new()`, schedules, events/messages flowing between multiple systems, spawned entities with several components, or access through `tests/` and crate public API, make it integration test.

Keep unit tests narrow: one behavior, one module, minimal setup, direct assertions. Test private implementation details only when they are invariant under test, such as corruption-prone packed storage layout; otherwise verify behavior through module public methods.

## Integration tests

Use integration tests for automated behavior that crosses module or system boundaries. Build the smallest Bevy `App` that proves behavior:

- Add only needed plugins, systems, resources, and messages.
- Spawn entities directly instead of running full startup.
- Prefer direct systems for one behavior path.
- Use plugins only when testing plugin wiring or ordering.
- Avoid `DefaultPlugins` unless the test explicitly needs window, rendering, or asset behavior.

Assert deterministic ECS state: components, resources, events/messages, spawned/despawned entities, or other world state. Do not assert through screenshots, logs, wall-clock timing, or human/LLM interpretation.

Use `app.update()` or a small helper such as `run_frames(&mut app, n)` for deterministic progression. Avoid sleeps and wall-clock waits.

Headless integration tests run in normal `cargo test`. Mark only slow, window, or local-debug checks with `#[ignore]`. Every ignored check must state why it is ignored and how to run it.

## Input tests

Prefer Bevy-level input simulation:

- Mutate `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>`.
- Send Bevy events/messages such as mouse wheel or app messages.
- Set cursor/window state only if system reads it.
- Run `app.update()`.
- Assert resulting components, resources, entities, or messages.

Do not use OS-level input automation for normal tests.

Keep helpers small. Start with generic helpers such as `run_frames`, `press_key`, `press_mouse`, or `send_mouse_wheel`. Add domain scenario helpers only after repetition appears.

Do not build a large test harness before tests need it. Add the smallest helper that removes real duplication in current tests.

## Screenshot checks

Screenshot checks are manual, temporary debugging aids for a specific visual purpose. Add them only for rendering, window, or UI problems where ECS assertions do not explain the issue, or when the user asks for visual evidence.

Screenshot checks are not automated integration tests and are not an acceptance gate. They must be ignored by default and must not require normal `cargo test`, human inspection, or LLM interpretation.

Before handoff, remove screenshot checks or convert the learned behavior into an automated unit or integration test. Leave one only if the user explicitly wants a local visual debug harness.

## Test data

Create needed data in code. Avoid loading assets or config files unless behavior under test is file loading, asset lookup, or rendering.

## Testing hard-to-reach behavior

When behavior is trapped inside a large Bevy system, extract the decision or calculation into a small pure function or module. Unit-test that function, then keep one integration test for system wiring if needed.
