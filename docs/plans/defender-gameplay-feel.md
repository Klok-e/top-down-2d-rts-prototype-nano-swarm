# Defender Gameplay Feel Hard-Cutover Plan

## Goal

Make Defender combat and sustain readable without changing intent capture semantics or the stable cell-holder doctrine.

The finished default skirmish must show a battle developing over seconds, staggered local recharge, bounded crowd motion, and deterministic outcomes. Defender paint remains destructible through capture. Supported holders remain attached to their assigned Defend cells; hostile pressure attracts idle, new, or replacement Defenders instead of retargeting active holders.

## Status

Complete as of 2026-08-03. The hard cutover, focused behavior tests, authored-default headless flow, offscreen screenshot evidence, real-process headless playtest, and performance validation are complete. The baseline symptoms below are retained as historical context for the decisions and acceptance ranges.

[ADR 0015](../adr/0015-territory-wide-defender-response.md) supersedes this plan's stable cell-holder and same-cell Charger doctrine for subsequent Defender work. The completion evidence remains a historical record of the implemented 2026-08-03 baseline.

## Completion Evidence (2026-08-03)

- Focused behavior coverage passes for combat, charge pulses, local Charger capacity, lease suspension/resumption, exact flank paint, and final movement speed. The full suite currently reports 442 passing tests; the playtest target reports 21 passing tests and one GPU-only ignored test.
- The authored-default headless flow starts through the scenario startup functions, verifies the primary Defend contest after the opponent's authored cadence, runs through contact plus 180 and 900 fixed ticks, and checks finite transforms and charge, the fixed-tick speed bound, per-Charger load bounds, holder retention, and health/capture progress. The minimal economy deterministically latches `MatchOutcome::Victory` when the opponent recovery check fails at the authored advance; the test asserts that exact result instead of accepting an arbitrary terminal outcome. The focused synthetic front remains separate coverage for staggered local sustain and charge return.
- Offscreen screenshot flow passes 18/18 ignored tests (`cargo test --test screenshots -- --ignored`, exit 0). The retained artifacts are `/home/dima/Desktop/top-down-2d-rts-prototype-nano-swarm/target/playtest-screenshots/defender_combat_early.png`, `/home/dima/Desktop/top-down-2d-rts-prototype-nano-swarm/target/playtest-screenshots/defender_combat_rotation.png`, and `/home/dima/Desktop/top-down-2d-rts-prototype-nano-swarm/target/playtest-screenshots/defender_combat_late.png`. Early shows both colored fronts and local Charger bars on contested paint; rotation retains holders while local service proceeds; late shows losses and capture progression consistent with ECS state.
- Real headless process command was `cargo run -- --headless --agent-socket --width 1280 --height 720` with a private `XDG_RUNTIME_DIR` and exit status 0. Fixed-tick state queries recorded an in-progress primary contest around tick 598, an evolving front with the player Defender cohort depleted around tick 1624, and the expected player Production Collapse defeat with the opponent still healthy around tick 4731. Returned screenshots were inspected, shutdown exited cleanly, and the private runtime/socket/process were removed after validation. No OS or compositor window was created.
- Final command evidence: `cargo fmt --all` exit 0; `cargo clippy --all-targets -- -D warnings` exit 0; `cargo test` exit 0 with 442 passed, 21 playtests passed, and one GPU-only playtest ignored; `git diff --check` exit 0. Latest `cargo bench --bench swarm_acceptance` estimates are steady Defend 6.3481–6.6674 ms/frame, exhausted Gather 5.5464–5.8027 ms/frame, and sparse distant Gather 3.9382–4.1681 ms/frame. All remain below the 16.7 ms frame budget.

## Baseline Problems (resolved)

- Combat applies damage every 60 Hz fixed tick. Equal full-charge Defenders kill each other in about 0.33 seconds; one Defender removes a 100-health structure in about 0.17 seconds.
- Charge reaches the rotation threshold in about 1.67 seconds. Identically initialized Defenders rotate together.
- A Defender consumes one mineral per charging tick. Sustain costs about 6.67 minerals per second before travel.
- `MAX_DEFENDERS_PER_CHARGER` controls structure demand but does not limit live assignments. A cohort can select one Charger simultaneously.
- Charger selection is global. A Defender may abandon its front for a distant Charger belonging to another Defend cell.
- Separation accumulates once per neighbor after direct movement, without a final speed clamp. Dense crowds may jitter or move faster than `bot_speed`.
- The obsolete per-Defender scoring allocator remains in source although production uses regional allocation. It implies retargeting behavior that is not registered.
- Default flank paint creates stable holders far from the authored primary clash, making correct lease behavior look broken.
- Existing tests prove transitions but do not constrain readable timing, default-scenario continuity, or combined combat/recharge behavior.

## Fixed Product Decisions

These decisions are implementation inputs, not questions for the implementing agent.

1. Preserve binary Defend paint, contest, withdrawal, capture, and destruction semantics.
2. Preserve shared Nanobot maximum health at 100. Tune attack cadence and damage rather than giving Defenders special health.
3. Preserve simultaneous combat resolution: mutually lethal attacks in one combat snapshot kill both participants.
4. Preserve stable active leases from ADR 0007 and ADR 0009. Threat does not pull a supported holder from another valid cell.
5. Restrict recharge to an operational, supplied Charger in the Defender's assigned Defend cell.
6. A Charger serves at most three en-route or charging Defenders concurrently. Each Defend cell also rotates at most half its associated cohort at once, rounded down with a minimum allowance of one. Charger capacity alone does not guarantee front continuity.
7. If no local Charger or rotation-cohort slot is available, a low-charge Defender continues holding. It may weaken, reach empty charge, and take health damage. It must not walk to another cell's Charger.
8. Returning Defenders request lease resumption through the regional allocator. They do not displace a replacement that already owns useful capacity.
9. Bound all composed movement, including separation, by `GameSettings.bot_speed`. Keep the current effective direct speed of 300 world units per second during this change.
10. Remove dormant per-Defender allocation code. Do not retain a feature flag or parallel path.
11. Remove the disconnected default flank Defend paint rather than teaching active holders to churn toward the primary battle.
12. Keep pathfinding, patrols, cross-cell pursuit, Attack intent, VFX overhaul, Maintenance tuning, and Production redesign out of scope.

## Target Tuning

The constants below are the starting contract. Derive exact boundary assertions from the implemented pulse order; do not silently loosen player-facing ranges to fit the code.

| Mechanic | Target | Acceptance range |
|---|---:|---:|
| Simulation rate | 60 fixed ticks/s | unchanged |
| Defender attack interval | 15 ticks / 0.25s | exact |
| Full-charge Defender damage against full-charge Defender | 5 HP/hit | exact |
| Equal Defender duel | about 300 ticks / 5s | 285-315 ticks |
| One Defender against 100-health structure | about 5s | 3-8s |
| First valid hit | immediate or within one interval | <=15 ticks |
| Charge drain | 0.00025/tick | exact |
| Full charge to 0.5 rotation threshold | about 2,000 ticks / 33.33s | 1,999-2,001 ticks |
| Recharge pulse interval | 10 ticks | exact |
| Charge gained per supplied pulse | 0.03 | exact |
| Mineral cost per supplied pulse | 1 | exact |
| Rotation threshold to full | about 190 ticks / 3.17s | 3-5s |
| Minerals per normal 0.5-to-full rotation | about 19 | exact after pulse-order test |
| Empty-charge damage | 1 HP every 6 ticks | exact |
| Empty 100-health Defender to death | 600 ticks / 10s | exact |
| Charger concurrent users | 3 | hard maximum |
| Movement displacement | `<= bot_speed + epsilon` per fixed tick | hard maximum |

Expected complete-cycle sustain is roughly 31 minerals per Defender-minute before travel and boundary rounding: about 19 minerals over about 2,190 fixed ticks. Record the exact derived value after implementation.

## Implementation Sequence

Follow red-green-refactor one vertical slice at a time. Do not create all failing tests at once.

### 1. Establish combat cadence

Files:

- `src/nanobot/combat.rs`
- `tests/behavior/combat.rs`

Work:

- Add deterministic per-Defender fixed-tick attack cooldown state.
- Target acquisition and pursuit continue every fixed tick; only damage delivery is gated.
- A newly valid target may be hit immediately. Reset cooldown only after a delivered attack.
- Preserve the existing simultaneous damage snapshot and deterministic target ordering.
- Keep current charged attack/defense values so equal Defenders take 5 damage per hit.
- Apply an explicit structure damage factor so a single full-charge Defender takes approximately five seconds to remove a 100-health structure.
- Remove the every-tick damage path and stale timing comments.

Acceptance:

- Equal Defenders remain alive through tick 284 and resolve between ticks 285 and 315.
- Reversing entity spawn order produces the same health timeline and outcome.
- Pursuit targets update between attack pulses.
- Mutually lethal attacks remain simultaneous.
- Nanobots remain preferred over hostile structures where existing priority rules require it.

### 2. Replace charge timing and material coupling

Files:

- `src/nanobot/charge.rs`
- `tests/behavior/charger.rs`
- `tests/behavior/charger_planned.rs`
- `tests/behavior/terminal_logistics_priority.rs`

Work:

- Replace per-tick refill/material consumption with one atomic supplied pulse every 10 ticks.
- One pulse consumes one Charger mineral and grants 0.03 charge. No mineral means no charge grant.
- Track pulse progress in `ChargerProgress` or a dedicated component.
- Rewrite `minerals_to_fully_charge` around the canonical pulse calculation.
- Drain field charge by 0.00025 each fixed tick.
- Apply one HP damage every six empty-charge ticks instead of two HP every tick.
- Keep `MAX_CHARGE = 1.0`, low threshold `0.5`, weakened threshold `0.3`, and Charger buffer capacity `60` unless exact conservation tests prove three normal rotations cannot complete due to rounding. If capacity changes, document the reason and new three-rotation margin.
- Delete superseded refill, drain, starvation, and material constants.

Acceptance:

- Full charge rotates on the first fixed tick whose post-drain `f32` value is at or below 0.5, expected within ticks 1,999-2,001. Record the observed boundary; do not add a broad epsilon that changes field endurance.
- A Defender starting at 0.5 inside a supplied Charger remains charging for at least 180 ticks and finishes within 300 ticks.
- Exact mineral debit matches helper math and `ResourceLedger` conservation.
- After becoming empty, a Defender takes exactly 100 damage pulses and dies 600 fixed ticks after the first six-tick grace interval begins. Define and test whether the first empty fixed tick starts or advances that interval.
- Weakening remains reachable when local sustain is unavailable.

### 3. Enforce local, capacity-aware Charger assignment

Files:

- `src/nanobot/charge.rs`
- `tests/behavior/charger.rs`
- optionally `tests/behavior/defender_sustain.rs`, registered in `tests/behavior.rs`

Selection contract:

1. Same swarm.
2. Operational.
3. Supplied.
4. Charger cell equals the held Defend cell.
5. Current en-route plus charging load is below three.
6. Current en-route plus charging cohort is below `max(1, associated_cohort_size / 2)` for that Defend cell.
7. Rank by distance, then stable entity ID.

Work:

- Store the source Defend cell in `ChargerAssignment`; it remains the cohort identity while `DefendHold` is suspended.
- Derive Charger load and source-cell cohort load from assignment/progress/hold components rather than a persistent mutable counter.
- Process rotating Defenders in stable entity order.
- Increment in-system temporary Charger and cohort loads immediately when assigning so same-tick contenders cannot overbook.
- Count `ChargerProgress` only through its retained assignment; never double-count one Defender.
- Rename global-nearest helpers to describe local and capacity-aware behavior.
- Make assignment cleanup one path. Charger destruction, empty supply, invalid ownership, Defender death, lost source-cell visibility/participation, or movement cancellation releases capacity and requests lease resume deterministically.
- A Charger itself is not captured; Defend paint is. Cancelling because the source Defend cell becomes invalid must remove assignment, progress, and movement before allocator reacquisition.
- Treat an en-route assignment with no movement component and outside Charger radius as cancelled; the existing stuck timeout must not leave a permanent reservation.
- A low-charge Defender with no valid local Charger or cohort slot retains `DefendHold`. Do not create a waiting movement state.

Acceptance:

- A capacity-isolation fixture with at least six associated Defenders and one Charger produces exactly three Charger assignments/progress states.
- For the normal three-holder front, only one rotates initially and two remain. For six associated Defenders, no more than three rotate even when multiple Chargers exist.
- Completing or cancelling one rotation admits the next eligible holder deterministically.
- A closer supplied Charger in another cell is rejected.
- Foreign, empty, degraded, and destroyed Chargers are rejected and never leak capacity; capture/loss of the source Defend cell cancels its old cohort assignments.
- Material cannot underflow under same-tick contention.
- Six synchronized holders never all abandon one cell; at least three remain even when automatic construction supplies multiple Chargers.
- Stuck or externally cancelled movement cannot retain a Charger slot.

### 4. Bound composed movement

Files:

- `src/nanobot/consts.rs`
- `src/nanobot/move_system.rs`
- `tests/behavior/defend_zone.rs`

Work:

- Clamp final finite movement vector to non-negative `GameSettings.bot_speed` immediately before integration.
- Derive facing from clamped movement.
- Preserve deterministic coincident-pair direction.
- Apply the same cap to every Nanobot role; do not add Defender-only movement semantics.
- Keep direct speed numerically unchanged. Per-second conversion is a separate refactor.

Acceptance:

- Direct movement plus dense-neighbor separation never displaces farther than `bot_speed + 1e-4` in one fixed tick.
- Coincident crowds remain finite and deterministic.
- Separation alone cannot expel a holder repeatedly across its cell boundary.
- Gather, Haul, Build, Maintenance, and Charger movement retain endpoint behavior.

### 5. Complete allocation hard cutover

Files:

- `src/nanobot/defend.rs`
- `src/nanobot/allocation/runtime.rs`
- `src/nanobot/allocation/lease.rs`
- `src/nanobot/spatial_pressure.rs` only where symbols become unused
- `tests/behavior/defend_zone.rs`
- `tests/behavior/regional_allocation.rs`

Work:

- Remove dormant `defender_assignment_system` and its scoring, self-exclusion, home-radius, and hysteresis helpers/tests.
- Remove `CellDensity`, its system, and crowding helpers only if `rg` confirms no remaining idle-spread or other consumer.
- Keep regional projection, invalidation, acquisition, arrival, and hold lifecycle as the single path.
- Keep threat pressure as additional projected demand.
- Keep supported holders stable across pressure changes.
- Ensure a charging holder stops satisfying active Defend capacity so a replacement may acquire work.
- Ensure a returning Defender resumes only through allocator capacity rules.

Acceptance:

- No dead alternate allocator remains.
- Active supported holder survives pressure changes without retargeting.
- New pressure attracts an idle or new Defender.
- Charging holder permits replacement.
- Returning holder cannot displace a valid replacement.

### 6. Make the default fight exercise the model

Files:

- `src/scenario.rs`
- scenario assertions in `src/scenario.rs`
- affected screenshot/playtest fixtures

Work:

- Remove disconnected flank Defend paint for both swarms.
- Keep primary fronts, unit counts, deposits, and opponent advance cadence unchanged initially.
- Update assertions that encode the old flank geometry.
- Run the default-flow playtest before considering any further scenario tuning.
- If three initially full Defenders overfill one baseline cell cosmetically, rely on separation/allocation first; do not add new intent cells merely to distribute sprites.

Acceptance:

- No stable holder is authored into a permanently irrelevant flank.
- First opponent advance produces a readable primary contest.
- Both sides survive at least 180 ticks after contact.
- Combat materially progresses or resolves by 900 ticks after contact.
- The Defender-feel acceptance does not accept an arbitrary terminal result. In the minimal authored headless stack, the opponent's deterministic recovery failure latches `Victory` at the authored advance; the playtest asserts that exact result and continues its combat checks through +900. The player does not latch Production Collapse in this flow; the full real-process run separately covers the later player-collapse outcome.

### 7. Refresh domain and technical documentation

Files:

- `CONTEXT.md`
- `docs/adr/0007-defender-spatial-pressure.md`
- `docs/performance/regional-allocation.md`

Work:

- Sharpen Charger language: finite local service belonging to a Defend cell; cut-off holders do not use remote Chargers.
- Keep numeric constants out of `CONTEXT.md`.
- Amend ADR 0007 timelessly so regional allocation from ADR 0009 is the mechanism and stable-holder/threat-pressure consequences remain the decision.
- Do not create a new ADR unless implementation reveals a genuinely hard-to-reverse, surprising trade-off not already covered.
- Refresh regional-allocation benchmark date/results after dead per-Defender work is removed.
- Do not edit `docs/agents/testing.md`; human approval is required.

## Comprehensive Validation

### Focused red-green tests

Add or update these named tests. Capture the initial failure and focused passing command for the implementation handoff.

`tests/behavior/combat.rs`:

- `equal_full_charge_defenders_resolve_in_readable_ttk_window`
- `combat_damage_only_occurs_when_cooldown_is_ready`
- `simultaneous_lethal_attacks_still_exchange`
- `pursuit_updates_between_attack_pulses`
- `single_defender_does_not_delete_structure_before_readable_window`

`tests/behavior/charger.rs` and optional `defender_sustain.rs`:

- `full_defender_holds_until_field_endurance_threshold`
- `low_defender_recharges_in_readable_bounded_time`
- `normal_rotation_consumes_exact_minerals`
- `empty_unsupported_defender_dies_after_grace_period`
- `defender_uses_charger_in_held_cell`
- `remote_charger_does_not_pull_defender_off_front`
- `charger_never_serves_more_than_maximum_concurrent_defenders`
- `released_charger_slot_is_claimed_deterministically`
- `invalidated_assignment_releases_capacity_and_resumes_lease`
- `material_is_not_overdrawn_under_charger_contention`
- `defend_cell_retains_holders_during_charge_rotation`

Movement/allocation tests:

- `combined_direct_and_separation_velocity_is_clamped_to_bot_speed`
- `coincident_crowd_velocity_remains_finite`
- `crowded_holder_remains_inside_supported_cell`
- `active_holder_keeps_lease_when_other_cell_pressure_rises`
- `charging_holder_allows_replacement_without_displacement_on_return`

Use fixed tick counts, explicit initial HP/charge/positions, deterministic swarm/entity ordering, and assertions at `N - 1` plus `N`. No sleeps or wall-clock assertions.

### Headless scripted playtest

Add `tests/playtest/defender_feel.rs`, register it in `tests/playtest.rs`, and name the flow:

```text
default_front_has_readable_combat_and_staggered_sustain
```

Use the near-runtime plugin stack without Winit. Observe the authored scenario from before contact through at least one recharge cohort and subsequent return/replacement decision.

Assert:

- first contact occurs inside a bounded window;
- both swarms have live Defenders 180 ticks after contact;
- aggregate health has decreased without instant front deletion;
- no Charger exceeds three assigned/charging Defenders;
- no fixed-tick displacement exceeds configured speed;
- no same-cell cohort fully evacuates for recharge;
- transforms and charge values remain finite and bounded;
- contest/capture materially progresses by 900 ticks after contact;
- match outcome and population remain consistent with collapse rules.

### Offscreen screenshot evidence

Add `screenshots/defender_combat_readability.rs` and register it in `screenshots/main.rs`.

Capture at least:

1. `defender_combat_early`: both fronts and contested paint visible.
2. `defender_combat_rotation`: active holders remain while local Charger and per-cell cohort limits are both satisfied.
3. `defender_combat_late`: losses/capture progression visible and consistent with ECS state.

After every screenshot callback resume, reassert camera and world preconditions because rendering advances gameplay. Open every PNG and record visual facts; artifact existence is not evidence. Update old screenshot timing assumptions only where the new cadence invalidates them.

### Real-process headless playtest

Never create an OS/compositor window. Launch the actual binary with a private runtime directory:

```bash
nano_swarm_runtime_dir="$(mktemp -d)"
chmod 700 "$nano_swarm_runtime_dir"
XDG_RUNTIME_DIR="$nano_swarm_runtime_dir" cargo run -- --headless --agent-socket --width 1280 --height 720
```

From a separate shell, export the printed directory explicitly, then use only the supported client:

```bash
export XDG_RUNTIME_DIR=/tmp/the-created-directory
python3 scripts/nano_swarm_control.py hello
python3 scripts/nano_swarm_control.py state
python3 scripts/nano_swarm_control.py screenshot --name defender-baseline
python3 scripts/nano_swarm_control.py wait --fixed-ticks 180
python3 scripts/nano_swarm_control.py state
python3 scripts/nano_swarm_control.py screenshot --name defender-early
python3 scripts/nano_swarm_control.py wait --fixed-ticks 2220
python3 scripts/nano_swarm_control.py state
python3 scripts/nano_swarm_control.py screenshot --name defender-late
python3 scripts/nano_swarm_control.py shutdown
```

Record returned state milestones. Inspect all returned image paths. Confirm readable losses, retained front coverage where the match survives long enough, absence of crowd launching/cross-map retreat, and agreement between screenshots and state. The real default process may resolve combat before the first 2,000-tick rotation; deterministic behavior/playtest and offscreen fixtures remain required proof of local recharge. Confirm zero exit, socket removal, process removal, and no compositor connection. Remove the private runtime directory only after validating the exact path and process exit.

### Performance validation

Run:

```bash
cargo bench --bench swarm_acceptance
```

Record all three 5,000-bot Criterion cases (`steady_defend_frame`, `exhausted_gather_frame`, and `sparse_distant_gather_frame`) and compare their whole-`app.update()` estimates with the prior run and `docs/performance/regional-allocation.md`. The current benchmark does not measure p95, allocation, or separation independently; do not claim those metrics without first adding instrumentation. Require no meaningful whole-update regression without explanation.

### Final repository gate

Run in this order:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test screenshots -- --ignored
cargo bench --bench swarm_acceptance
git diff --check
git status --short
git diff --stat
git diff
```

Review the full diff for correctness, timeless comments, dead code, accidental unrelated edits, and generated artifacts. Preserve the pre-existing dirty worktree. Do not commit or push without explicit authorization.

## Implementation Handoff Requirements

Final implementation handoff must include:

- selected constants plus derived ticks, seconds, attacks per second, minerals per rotation, and minerals per Defender-minute;
- all changed and deleted files, separated from pre-existing worktree changes;
- exact new/updated test names;
- RED evidence then GREEN result for every tracer-bullet slice;
- final fmt, Clippy, full test, screenshot, benchmark, and diff-check results;
- real-process launch/control transcript summary, fixed-tick state milestones, exit status, socket cleanup;
- absolute screenshot artifact paths and inspected visual facts;
- confirmation that no OS/compositor window was created;
- deviations from this plan with reasons;
- residual gameplay, performance, and system-ordering risks;
- reviewer findings and final working-tree status.

Before declaring completion, use an independent read-only reviewer on the implementation diff. Resolve real findings with a focused regression first, rerun affected focused tests, then rerun the complete final gate.
