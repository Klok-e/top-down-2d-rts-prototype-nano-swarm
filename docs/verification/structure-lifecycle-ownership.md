# Structure lifecycle ownership

Verified 2026-09-05 against `dae230b`. This refactor implements the approved lifecycle ownership design following issues #59–#65; it preserves ADR-0016/0017 gameplay policies.

## Module responsibilities

- `nanobot/structure_lifecycle.rs` owns private construction work, reservations, validation state, evacuation, activation, cancellation, and schedule registration. Allocation requests a complete Worker-assignment transition and reads available work. Finished construction cannot accept another Worker.
- `physical_world.rs` is the single ECS adapter interpreting footprint consequences: traversable, entry barred, or solid. Navigation uses its obstacle snapshot; movement uses `movement_clear`; production exits use `can_occupy`. Consumers do not inspect Clearing validation internals. Entry barriers prohibit new bodies while allowing existing occupants to leave.
- `structure_overlay.rs` reads construction progress and owns cancellation residue and animation. Residue has no structure, reservation, or collision identity.
- `StructureLifecycleSet::Commit` publishes at the end of the construction chain, after movement, allocation acquisition, and production. Deferred transitions are applied and navigation refreshed before observers after that phase or consumers in the next fixed tick see the resulting capacity. Initial `FixedFirst` refresh also incorporates externally authored world changes. The phase changes neither navigation work allowance nor production's retained-output policy.

The old `clearing.rs` and consumer-side Clearing predicates are removed. Completed stockpile/facility/charger payloads still express operational capacity; there is no second active-state flag to synchronize. Public authored-state constructors remain available for explicit fixtures/restoration; runtime assignment and completion use lifecycle-owned transitions.

## Verification

New behavior checks cover pending validation versus entry barriers, occupant escape versus new-body placement, same-tick activation and cached navigation agreement, continued entry restrictions while changed-layout revalidation has no budget, and refusal to recruit Workers after construction finishes. The cancellation regression now checks both immediate body placement and a budgeted route into the released site without another ECS tick. The activation assertion was inverted and observed failing before restoration.

Existing lifecycle, access, crowd, production accounting, and gameplay outcome assertions remain. This is an internal refactor; existing scripted playtests `authored_charger_planning_and_maintenance_follow_observed_service_need` and `authored_default_scenario_reaches_primary_defend_contest` cover the affected construction/service and economy flow and both pass.

| Check | Result |
| --- | --- |
| Formatting and diff checks | Passed |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo test` | 363 unit + 496 behavior + 38 playtests passed |
| `cargo test --test playtest -- --ignored` | Both GPU playtests passed |
| `cargo test --test screenshots -- --ignored` | All 36 offscreen trials passed |
| Independent Standards and Spec reviews | No confirmed defects or violations |

Inspected fresh captures in `target/playtest-screenshots/`: `construction_clearing_activated.png` shows the Defender outside the completed stockpile; `construction_cancellation_collapse.png` and `construction_cancellation_fade.png` show the red outline shrinking behind the unchanged Defender; `production_exit_released.png` shows one Hauler in the vacated exterior position below the facility. These check preserved rendering; ECS assertions establish lifecycle and collision timing. No OS/compositor window was created. No separate real-process session or scale benchmark was run for this refactor.

Review noted two nonblocking limitations: authored-state construction methods permit explicit partial fixtures, and spatial consumers independently allocate snapshots using the same interpretation. No performance improvement is claimed. Final logs and the assertion-inversion evidence are retained locally under `target/lifecycle-verification/`. `docs/agents/testing.md` was not modified.
