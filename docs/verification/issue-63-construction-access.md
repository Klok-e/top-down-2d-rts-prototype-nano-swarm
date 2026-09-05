# Issue #63: preserve friendly construction access

Verified 2026-09-05 against starting commit `79338b1`. Issue #63 is unblocked by the closed #59. The shared access implementation was already present in `dae230b` and the lifecycle consolidation in `28226ec`; this follow-up closes its missing simulation acceptance coverage. No production behavior changes were necessary.

## Deterministic coverage

Seven new tests in `tests/behavior/construction_access.rs` exercise real automatic planners and budgeted Navigation through the approved minimal simulation seam:

- `automatic_construction_considers_both_passages_when_another_plan_closes_one`: a literal wall has two 144-unit openings. A 72-unit Charger can close one while the other stays open. With a Source Stockpile plan closing the other opening, Charger demand chooses an alternative instead of severing the friendly connection.
- `automatic_construction_does_not_require_disconnected_friendly_networks_to_join`: a full-height wall already separates friendly Stockpiles. Construction still chooses the reachable preferred site on the builder's side.
- `automatic_construction_may_close_an_enemy_only_connection`: the preferred Charger site may close the only passage between enemy Stockpiles.
- `{source,sink,charger,production}_planner_waits_for_builder_access_for_both_swarms`: each real planner receives its demand while its only Worker is behind a full-height wall. No plan appears during 500 explicit updates; removing the wall permits exactly one correctly typed, owned, snapped plan within 200 updates. Both player and opponent ownership are exercised. The Source case seeds an existing Gather assignment; the Production case uses the existing isolated priority-demand seam. These are placement tests, not end-to-end demand allocation or Worker travel tests.

The existing alternate-site simulation test independently routes between friendly Stockpiles with the chosen plan made solid. Existing public-navigation and access unit tests cover clearance, Gather-eligible deposits, baseline connectivity, and shared pending work budgets. Placement and final completion continue to share `AccessLayout::check`; ordinary plans remain physically traversable. The existing `construction_access` offscreen scenario exercises the full app's demand-to-plan flow. No scripted playtest was changed because this follow-up adds acceptance tests without changing player-facing runtime behavior.

## Mutation evidence

Temporary production mutations proved the new assertions reject concrete wrong behavior:

- Ignoring other plans in hypothetical completion fails the two-passage test.
- Requiring disconnected baseline endpoints to become connected fails the disconnected-network test.
- Protecting enemy endpoints fails the enemy-obstruction test.
- Bypassing builder reachability fails all four planner tests.

The first builder mutation exposed a deficient Source fixture: its friendly Stockpile wall also satisfied Source demand. Making the wall explicitly opponent-owned removed that unrelated suppression; the corrected Source test then failed on bypassed builder validation. All mutations were restored before final verification. Mutation logs are under `target/issue-63-verification/`.

## Offscreen evidence

`cargo test --test screenshots -- --ignored` passed all 37 trials. Fresh inspected captures under `target/playtest-screenshots/`:

- `construction_access_open_passage.png`: two brown wall sections leave an opening between green friendly Stockpiles; the blue Defender occupies the opening and the white Worker stands to the lower left.
- `construction_access_safe_alternative.png`: a Charger plan outline appears to the left of the lower wall section, below the left Stockpile. The opening remains visibly unobstructed. The full-app scenario also asserts that the selected plan stays at least 144 units from the unsafe central site.

## Real headless runtime

A fresh `cargo run -- --headless --agent-socket --width 1280 --height 720` process used a private mode-0700 temporary runtime directory. `scripts/nano_swarm_control.py` set the camera, read state, captured the start, painted Defend at (1, 1), waited 300 fixed ticks, read state, captured again, and shut down. The reproducible driver is `target/issue-63-verification/runtime_check.py`; responses are in `runtime-commands.jsonl`.

State advanced from tick 6 to 321. Both swarms remained in progress with W4/H2/D3; player minerals increased from 0 to 98. The inspected `issue63_start-391678-000000.png` shows separated starting bodies beside production and a Stockpile plan. `issue63_after_paint-391678-000001.png` shows completed Source/Sink Stockpiles, Workers outside the deposit, Haulers between buffers, and Defenders advancing into the painted area. Its later capture displays 104 minerals. This is an ordinary app-wiring and construction-flow check; forced unsafe-placement decisions are proved by the dedicated simulation and full-app offscreen scenarios above.

Shutdown at tick 332 returned exit code 0. Socket removal, released lock, and temporary-directory cleanup passed. No OS window or compositor automation was used.

## Checks and review

- `cargo fmt`, `git diff --check`, and `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 363 unit tests, 512 behavior tests, and 45 scripted playtests passed. Two GPU playtests and 37 screenshot trials are ignored by default; the screenshot trials passed separately as above.
- Independent Standards review: no documented violations or blocking findings. A non-blocking taste observation suggested separating planner-specific setup from the shared builder-access scenario; the explicit kind-based fixture remains local to these four tests.
- Independent Spec review: no missing requirements, incorrect behavior, or scope creep identified in the acceptance follow-up.

Logs and runtime artifacts are retained under `target/issue-63-verification/` and are not committed. The 5,000-Nanobot performance and aggregate navigation acceptance remain issue #66 work.
