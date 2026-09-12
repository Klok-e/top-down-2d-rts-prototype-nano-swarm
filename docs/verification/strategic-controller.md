# Strategic Controller verification

Status: the sole-controller cutover and automated verification are complete. Human-tested novice difficulty remains unverified until a human plays Standard.

## Active contract

Standard and every automatically controlled swarm use the same Strategic Controller through `Controller::new`. The controller observes an immutable `GameState` and returns owner-scoped `Paint` and `Erase` intent edits. Plan lifecycle and work budgeting, strategy scoring, intent materialization and geometry, and private invariants are separated inside one module without policy traits, registries, or selection hooks.

The controller sustains one primary resource site, logistics, support, and a fighting force; mounts attacks; responds to pressure and lost support; retargets depleted resources; and pursues exposed structures and surviving Nanobots. It plans with a shared 100,000-work-unit allowance per review window. Owned-intent cleanup examines at most 256 active cells per review and carries a deterministic row-major cursor across reviews so a large foreign prefix cannot permanently hide stale owned intent.

AI Battle runs two copies of this controller on Standard or Flanks. All shipped scenarios use 90 construction work ticks, a 30-tick Defender attack interval, and 0.000125 Charge drain per tick. AI Battle records factual effective damage, population, economy, structures, outcomes, and deterministic controller telemetry. It has no automatic time cutoff and records explicit interruption without assigning an outcome.

The Timed controller, policy selector, side swapping, pacing selection, experiment mode, reserved layouts, scored-damage credit, controller wall-clock telemetry, and Python comparison runner are removed. [ADR-0023](../adr/0023-one-strategic-controller-policy.md) records this boundary. Earlier comparative work remains in the [research history](strategic-controller-research-history.md) as historical evidence rather than an active gate.

## Automated evidence

The public stale-intent regression first failed against the fixed-prefix cleanup scan, then passed after cursor-based cleanup. It places more than 256 foreign active cells before stale owned intent and also proves the foreign layer remains unchanged.

After the API and module cutover, all 29 private controller invariants, nine public Strategic Controller behavior tests, and five retained controller combat playtests passed. These cover resource and support invalidation, shared budgeting, terminal shutdown, real gathering and hauling, concurrent attack fronts, exposed structures, distant remnants, and natural Swarm Elimination. Resource loss, support loss, and the 100,000-unit shared allowance are public contract tests; their duplicate private checks were removed.

Repository-wide verification passed `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`: 383 library tests, 521 behavior tests, and 78 non-GPU playtests passed. The separately invoked ignored suites also passed all 45 screenshot tests and all three GPU/offscreen playtests. Representative faults that retained an exhausted primary deposit, ignored a lost key Charger, reset the allowance between reviews, and stopped effective-damage accumulation each made the corresponding public or recorder test fail before production behavior was restored and the tests passed.

## Real-process evidence

All runtime checks used a private mode-0700 `XDG_RUNTIME_DIR`, a process-local null ALSA device, the headless agent socket, and no window. Start, middle, and late screenshots were captured and inspected at original resolution. The late Standard player frame visibly contains its expanded economy and mixed force; the late AI Battle frame visibly contains both colors in motion, two facilities per side, and populations W5/H8/D13 versus W4/H11/D18 under the readable spectator-only HUD. The neutral, impact, and dense-volley combat frames remained visually distinct. Runtime processes shut down cleanly, released their locks, and removed their sockets.

Accelerated AI Battle ran to natural elimination without a cutoff. Standard seed 11 ended after 27,165 fixed ticks and 452.75 simulated seconds with `swarm_1_wins`; Flanks seed 11 ended after 17,665 ticks and 294.42 seconds with `swarm_1_wins`. Both schema-version-4 summaries recorded the shipped pacing, activity from both controllers, factual effective damage, and no scored-damage or controller wall-clock fields. Because both sides ran the same controller, these outcomes verify liveness and termination rather than relative policy superiority.

The final real-time Standard seed 11 run remained healthy through the inspected state at 9,475 ticks and 157.92 simulated seconds, with late captures continuing past 166 seconds. The player had 22 Nanobots and four facilities while the opponent had 38 Nanobots, two facilities, and had advanced from its starting corner toward the map center. The final real-time Standard-layout AI Battle remained in progress through 9,787 ticks and 163.12 simulated seconds; both controllers expanded, gathered, built seven structures, inflicted effective damage, and continued issuing intent edits. Explicit shutdown produced `status: interrupted` with no outcome, as required.

The retained evidence is under `target/sole-controller-proof/`; it includes both accelerated summaries and the final inspected real-time screenshot sequences and interrupted AI Battle summary. Human play remains the only unperformed check, so the controller's novice-facing difficulty is not claimed as verified.
