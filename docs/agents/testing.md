# Testing

This document defines repo-specific testing conventions. For the TDD loop itself, use the TDD skill.

## Commands

Run one focused test while developing:

```bash
cargo test <test_name>
```

Run the behavior or playtest integration target:

```bash
cargo test --test behavior
cargo test --test playtest
```

Before handoff, run:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

`cargo test` runs unit tests, behavior tests, and headless scripted playtests.

## File layout

```txt
src/
  lib.rs
  main.rs
  ...modules with #[cfg(test)] unit tests...

tests/
  common/
    mod.rs
  behavior.rs          # aggregate integration target
  behavior/
    *.rs              # focused behavior / acceptance tests
  playtest.rs         # aggregate integration target
  playtest/
    *.rs              # scripted player/runtime flows
```

Nested integration files import the shared seam explicitly:

```rust
#[path = "../common/mod.rs"]
mod common;
```

`tests/common/mod.rs` is the shared test seam introduced for issue #17. Behavior and playtest files use the `sim_app_with_*` builders and `spawn_*` helpers from there. New tests should follow the same pattern:

1. Import the common seam with `#[path = "../common/mod.rs"] mod common;` when the test needs shared helpers.
2. Replace local `build_app`, `cell_world_center`, and `spawn_*` helpers with canonical helpers in `common::`.
3. Pick the smallest `common::sim_app_with_*` builder that covers the test's plugin set.
4. Put specialised spawn helpers that need markers or non-default fields in `tests/common/mod.rs`, not duplicated in test files.
5. Add new files to the matching aggregate target (`tests/behavior.rs` or `tests/playtest.rs`) so Cargo discovers them.

Unit tests live beside the module they test. Automated integration tests live under `tests/behavior/`. Scripted playtests live under `tests/playtest/`.

## Unit tests

Use unit tests for behavior that belongs to one Rust module and can be checked without a Bevy `App`, ECS schedule, spawned world, or public crate boundary. Put them beside code they exercise.

Good unit-test targets in this repo:

- Local data encoding, masking, bounds, and validation rules.
- Module-owned indexing helpers and boundary checks.
- Pure math helpers such as coordinate conversion, distance checks, scoring, sorting, and clamping.
- Small local state transitions that only need normal Rust structs/enums.
- Extracted helper functions from systems when system logic is mostly calculation.

Prefer extracting pure functions over mocking Bevy queries, resources, commands, assets, or windows. Use fakes only at real module boundaries, not to simulate Bevy internals.

If test needs `App::new()`, schedules, events/messages flowing between multiple systems, spawned entities with several components, or access through `tests/` and crate public API, make it an integration test.

Keep unit tests narrow: one behavior, one module, minimal setup, direct assertions. Test private implementation details only when they are invariant under test, such as corruption-prone packed storage layout; otherwise verify behavior through module public methods.

## Behavior tests

Use `tests/behavior/*.rs` for automated behavior that crosses module or system boundaries. Build the smallest Bevy `App` that proves behavior:

- Add only needed plugins, systems, resources, and messages.
- Spawn entities directly instead of running full startup.
- Prefer direct systems for one behavior path.
- Use plugins only when testing plugin wiring or ordering.
- Avoid `DefaultPlugins` unless the test explicitly needs window, rendering, or asset behavior.

Assert deterministic ECS state: components, resources, events/messages, spawned/despawned entities, or other world state. Do not assert through screenshots, logs, wall-clock timing, or human/LLM interpretation.

Use `app.update()` or a small helper only after repetition appears. Avoid sleeps and wall-clock waits.

Headless behavior tests run in normal `cargo test`. Mark only slow, window, rendering, or local-debug checks with `#[ignore]`. Every ignored check must state why it is ignored and how to run it.

## Scripted playtests

For runtime/player-facing changes, add or update a scripted playtest when the change touches input, UI, camera, startup/plugin wiring, gameplay flow, or rendering-visible behavior. This is mandatory for bug fixes in those areas: reproduce the reported failure in a failing playtest before or alongside the fix, then prove the same playtest passes after the fix. Existing passing playtests are not enough unless one is explicitly updated or identified as covering the exact reported regression.

A handoff for runtime/player-facing fixes must state the playtest coverage by file and test name. If no scripted playtest was added or changed, the handoff must explain why the change is not player-facing or why an existing named playtest covers the exact bug. "cargo test passed" alone is not sufficient evidence.

Use `tests/playtest/*.rs` for flows that combine app wiring and player actions. Examples:

- Select an intent layer, paint/erase, and assert the intended layer changed.
- Press camera controls, advance frames, and assert camera state changed.
- Click UI controls by setting `Interaction::Pressed`, advance frames, and assert resources/components changed.
- Run a near-runtime plugin stack long enough to catch missing startup resources or ordering failures.

Script input through Bevy state, not OS automation:

- Mutate `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>`.
- Send Bevy events/messages such as mouse wheel or app messages.
- Set cursor/window state only if the system reads it.
- Set UI `Interaction` components for UI controls.
- Run `app.update()` and assert deterministic ECS state.

Headless scripted playtests run in normal `cargo test`. If a playtest needs a real window, GPU, or screenshot, keep it under `tests/playtest/`, mark it ignored, and explain how to run it.

### Screenshot evidence

Screenshots are a scripted playtest technique, not a separate test category. For changes affecting shaders, UI layout, materials, cameras, render targets, or other visual appearance, agents must produce screenshot evidence in addition to any deterministic ECS assertions.

Screenshot evidence must be inspected, not merely produced. The agent that produces screenshots must open/read the image artifacts, describe the relevant visual facts, and state whether they satisfy the acceptance criteria. For visual bug fixes, the verifier must independently inspect the screenshots before passing. If screenshots are ambiguous, improve the scripted setup or fail/needs-info with the exact blocker; do not pass on artifact existence alone. Rust pixel assertions are useful deterministic checks, but they do not replace agent inspection for visual bug fixes.

The preferred Bevy capture path is to spawn `bevy::render::view::screenshot::Screenshot::primary_window()`, observe `bevy::render::view::screenshot::save_to_disk(path)`, and wait for `ScreenshotCaptured` before continuing the scripted flow. Shared helpers that hide this ceremony may live in `tests/common/mod.rs` and may remain committed when they improve maintainability.

Use a temporary ignored playtest for screenshot investigations:

```txt
tests/playtest/temp_<thing>.rs
```

The temporary playtest should write artifacts under:

```txt
target/playtest-screenshots/
```

Remove temporary screenshot playtests before handoff unless the user explicitly asks to keep them or the rendering regression cannot be covered through ECS/state assertions. Durable screenshot-producing playtests may live under `tests/playtest/` only with a clear comment explaining why screenshot evidence is required. Do not commit screenshot image artifacts unless the user explicitly asks for committed baselines. If the environment lacks a usable window/GPU, report the exact blocker instead of treating it as a normal test failure.

## Test data

Create needed data in code. Avoid loading assets or config files unless behavior under test is file loading, asset lookup, or rendering.

## Testing hard-to-reach behavior

When behavior is trapped inside a large Bevy system, extract the decision or calculation into a small pure function or module. Unit-test that function, then keep one behavior or playtest test for system wiring if needed.
