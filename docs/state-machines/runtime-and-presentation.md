# Runtime and presentation state machines

[Inventory and execution boundaries](../state-machines.md)

## Headless application runner

`run_dynamic_headless` waits while plugins are being added, advances the global task pools, finishes plugin installation, and cleans up before entering the update loop. Each iteration updates the app, exits when `AppExit` is observed, or delays according to `HeadlessPacing`. Bounded pacing uses real elapsed time. Unbounded pacing advances one fixed simulation step per application update. The menu can pause virtual simulation while process frames and agent control continue. [Runner](../../src/runtime.rs), [headless pacing playtest](../../tests/playtest/headless_pacing.rs)

## Menu and pause

`ScenarioMenu.open`, `blocks_world_input`, and `was_paused_before_open` encode closed, open, and the closing frame that still consumes world input.

- `toggle` changes visibility and latches input suppression. `menu_keyboard` recomputes suppression each frame, retaining it through the closing frame.
- `synchronize_pause` captures the previous pause state once on opening, pauses virtual time, and restores that state on closing. It zeroes the already-computed frame delta whenever input is suppressed.
- Selection, start, and quit actions require an open menu. A successful Scenario start closes the menu, consumes that frame's world input, and clears the saved pause state.
- Camera movement clears its velocity while the menu blocks input. Camera zoom, brushes, and brush selection also honor the input gate.

Owners are `ScenarioMenuPlugin`, `menu_keyboard`, `menu_actions`, and `synchronize_pause`. [Menu implementation](../../src/ui/scenario_menu.rs), [camera input](../../src/fly_camera/camera_move.rs), [menu playtests](../../tests/playtest/scenario_menu.rs)

## Live tuning publication and editor

`TuningStore` owns the selected file path; `Tuning`, `TuningRevision`, and `TuningJournal` retain the published configuration, process revision, and accepted edit history. Startup strictly loads and validates the complete RON file before scenario construction. The default path is project-root `config/balance.ron`; `--tuning-file` overrides it.

- An edit validates a candidate against field bounds, native-unit representation, and cross-field relationships. An unchanged candidate returns the current revision.
- A changed candidate writes and syncs a separate temporary file, then atomically renames it into place. Failure removes the temporary file and leaves published values, revision, and journal intact.
- Successful persistence publishes the candidate, increments the revision, appends the current simulation tick and displayed old/new values, and synchronously runs reconciliation hooks. Hooks update all swarms. Scenario replacement retains the store, configuration, revision, journal, and editor.
- `TuningEditor` retains visibility, expanded groups, focused field, raw text, caret/selection, errors, and input-capture flags. F2 toggles visibility. Group activation toggles expansion; collapsing a focused group releases focus. Each numeric text change or Up/Down step attempts publication immediately. Rejected raw text remains visible with a field error; a successful edit clears that error.
- Focus captures keyboard input, including the frame that releases it. Escape releases focus first; without focus it reaches the scenario menu. Pointer input over the open panel is captured and scrolls its contents. The panel itself leaves virtual time running; the scenario menu hides it and supplies the pause boundary. Closing the panel releases focus and pointer capture while retaining expansion and field text/error state.

Owners are [tuning publication and storage](../../src/tuning.rs), [field metadata](../../src/tuning/values.rs), and [panel](../../src/ui/tuning_panel.rs). Verification covers [storage tests](../../src/tuning.rs), [agent tuning](../../tests/behavior/agent_tuning.rs), [panel playtests](../../tests/playtest/tuning_panel.rs), and [panel screenshots](../../screenshots/tuning_panel.rs).

## Scenario selection and replacement

`ScenarioSelection.current` identifies the running Scenario. `next_launch` records the saved choice, and `error` retains a failed selection or start result. Loading uses the saved value when available. A CLI Scenario can override the current launch.

- `select` writes a separate temporary file, syncs it, and renames it into place. Success updates `next_launch` and clears the error. Failure removes the temporary file and retains the previous choice for retry.
- `process_start_requests` drains start messages and uses the last request. It flushes deferred work and finalizes any battle recording before teardown. A finalization error retains the old ECS session and reports a retryable error.
- A successful finalization permits despawning session entities and resetting intent, ledgers, Nanobot resources, navigation, brush selection, input, simulation clocks, and camera. `SessionGeneration` advances and invalidates retained local caches.
- `populate_session` installs Scenario rules and entities, reachability, controllers, and any recording. Population failure is reported after teardown. Successful population refreshes the layer UI and closes the menu.
- The application shell, real clock, agent transport, and agent synchronization clocks outlive replacement.

Owners are `ScenarioSelection` and `SessionLifecyclePlugin`. [Selection persistence](../../src/scenario_selection.rs), [replacement](../../src/session_lifecycle.rs), [runtime launch](../../src/runtime.rs), [restart playtests](../../tests/playtest/session_restart.rs)

## Match Outcome

`MatchOutcome` explicitly stores `InProgress`, `Winner(SwarmId)`, or `Draw`. The implicit part is the one-way latch and its consumers.

- With outcomes enabled, `detect_swarm_elimination` recomputes participating swarms and survivors in `FixedLast`, after creation and Nanobot death settlement. Planned Structures are excluded from completed infrastructure.
- While in progress, fewer than two participants or multiple survivors keep the match running. One survivor wins, and zero survivors draw.
- Once terminal, later simulation changes leave the result latched. Player intent edits and Strategic Controller planning stop accepting work, and recording freezes its result. Simulation and presentation can continue.
- Session replacement resets the outcome. `update_match_banner_system` retains the last presented outcome to skip redundant UI work and updates visibility, label, and color when it changes.

[Elimination and outcome](../../src/nanobot/elimination.rs), [banner mirror](../../src/ui/match_banner.rs), [outcome tests](../../tests/behavior/swarm_elimination.rs), [banner tests](../../tests/behavior/match_banner.rs)

## Strategic Intent Plan

`Controller` owns `PlannerState`. Its optional current plan, review tick, work-window start, and accumulated budget encode initial selection, retained execution, scheduled review, urgent review, and budget deferral. `Objective` explicitly names the plan's objective.

- Each decision opens or resumes a bounded review work window. Validation checks the incumbent's deposit, support, attack obligations, and reachability. Exhaustion retains the incumbent and defers additional work.
- Regular reviews use the current `autonomy.controller_review_period_ticks` against the retained last-review tick; changing it does not reset the incumbent or accumulated bounded work window. Invalidated obligations can trigger earlier review. Between reviews a viable plan remains active.
- Candidate scoring retains a viable incumbent unless an alternative clears the switching margin. Materialization must finish a coordinated plan within the allowance before replacing it and emitting owned-intent edits.
- `retained_covered_movement` and `covered_stall_grace_used` form an attack submachine. Moving or progressing attacks inside existing owned Defend coverage can retain the plan. Progress resets grace. One stalled review can consume grace, and later stalled review permits reassessment. Losing coverage can invalidate the retained attack.
- Terminal Match Outcome stops the runtime adapter from calling the planner. A fresh Scenario installs fresh controllers and telemetry.

[Planner state and transitions](../../src/strategic_controller/lifecycle.rs), [plan validation](../../src/strategic_controller/strategy.rs), [runtime adapter](../../src/strategic_runtime.rs), [controller behavior tests](../../tests/behavior/strategic_controller.rs)

### Bounded owned-intent cleanup

Two independent optional cursors retain progress through obsolete owned paint and unreachable owned paint.

- Each scan resumes after its cursor, spends the shared work allowance, and stores the last completed cell when it reaches a budget or scan bound.
- Exhausting the ordered active-cell sequence clears the cursor so a later call starts another sweep. Foreign paint is visited without losing progress toward later owned cells.
- Grid revision rollback clears both cursors. Desired paint and cleanup share the planner's edit budget.

Owner is `intent_edits` with cursors held by `PlannerState`. [Cleanup](../../src/strategic_controller/materialize.rs), [controller coverage](../../tests/behavior/strategic_controller.rs)

## Battle recording and durable finalization

`RunStatus` explicitly stores in-progress, completed, or interrupted. `BattleRun` presence, its optional error, frozen `BattleCounters`, next sample time, and timing buffers add collection and persistence phases.

- `start_session` creates recording only when the Scenario requests it. Successful creation installs fresh counters. Creation failure requests error exit.
- Schema 6 snapshots native-unit tuning at battle start and remembers the journal offset and simulation start tick. Collection appends subsequent accepted edits with ticks relative to that battle, excluding previous-session edits. A replacement battle takes a fresh snapshot.
- Fixed ticks collect timing and first observed elimination ticks. Sampling writes factual state at simulation-time boundaries. Frame timing has a separate first-frame baseline and follows recording configuration.
- A terminal Match Outcome sets completed status, records the outcome, freezes counters, and writes the terminal sample. Successful headless completion requests normal exit.
- `finish_session` marks an active run interrupted and freezes counters before replacement. A failed write retains the run and error for another start attempt. A successful retry clears the error and permits replacement.
- `finish_on_exit` finalizes an active, error-free run as interrupted. Collection stops on terminal status or a retained error. Session startup replaces or removes the recording resource according to the new rules.

[Recording and finalization](../../src/battle_statistics.rs), [recording tests](../../tests/behavior/battle_statistics.rs), [battle runtime playtests](../../tests/playtest/battle_runtime.rs), [restart tests](../../tests/playtest/session_restart.rs)

## Agent control

The process owns these lifecycles independently of Scenario replacement. Protocol values and limits are specified by the agent-control guide. [Protocol and synchronization](../agents/agent-control.md)

### Server and connection

`AgentControlPlugin` takes its optional listener, socket guard, and lifecycle lock once during installation. `AgentControlServer` retains a stop flag and optional worker handle.

- Binding validates the private socket directory, obtains the lifecycle lock, and removes a stale socket only under that lock.
- The worker accepts one client, serves it, then returns to accept. Drop signals stop, joins the worker, and lets the guard remove its own socket inode.
- Each connection reads a framed line, parses a command, submits it to the bounded queue, polls its reply, writes a response, and repeats. Parse, queue, disconnect, timeout, read/write, and stop conditions either report an error or terminate that connection.
- Asynchronous requests carry their own atomic cancellation flag. Disconnect or response timeout can cancel pending work. Cancellation processing removes queued waits, deferred captures, and active capture entities.
- `read_control_line` switches from buffering to discarding after its size limit. The `too_large` latch persists until newline or EOF, while the absolute line deadline bounds incomplete input.

[Transport, framing, cancellation, and socket tests](../../src/agent_control.rs)

### Pending wait

`PendingWaits` membership and target frame/fixed-tick values encode a queued wait.

- `process_agent_requests` validates requested deltas and snapshots the process clock.
- `complete_agent_waits` replies only when both targets have been reached, then removes the entry. Cancellation removes the entry without completion.
- Frames can advance while virtual simulation is paused. A wait requiring fixed ticks continues waiting until those ticks occur. Scenario replacement resets simulation time while the agent clocks remain monotonic.

[Wait processing and tests](../../src/agent_control.rs)

### Pending screenshot

`DeferredScreenshotRequests`, `PendingScreenshots`, capture-entity membership, and optional `capture_clock` encode startup deferral, submitted capture, stamped frame, and completed readback.

- Startup requests defer while the agent frame clock is zero. A later `PreUpdate` submits the capture. Admission allows one deferred or pending capture.
- Submission validates the render target and output path, spawns a capture entity, and records its reply/cancellation context. `stamp_pending_screenshot_clocks` records the first submission clock.
- `complete_agent_screenshot` removes the pending entry, converts and saves readback, applies private permissions, and replies with capture and completion clocks. Conversion/save failures produce error responses.
- Cancellation removes pending ECS capture state. The completion path checks cancellation before conversion, before save, and afterward, removing a saved file if cancellation wins that race.

[Capture queue, completion, and tests](../../src/agent_control.rs)

### Synthetic button pulse

`PendingButtonReleases` records buttons set to explicit Bevy `Interaction::Pressed`. The next `PreUpdate` releases them before processing new commands. Menu actions and layer selection consume the press through their normal UI systems. Missing button entities are tolerated during release.

[Press and release](../../src/agent_control.rs), [input layer UI](../../src/ui/intent_layer_panel.rs)

## Intent rendering and presentation mirrors

`IntentGrid.render_dirty` and `projection_dirty` are independent consumer queues. Paint or erase that actually changes ownership advances the revision and marks both. The render mirror drains only its set. Session reset marks previously painted cells for clearing in the GPU buffer. [Intent state](../../src/intent.rs), [render propagation](../../src/zones/zone_brush.rs)

| Retained presentation | Transition and cleanup |
| --- | --- |
| Deposit visual | Missing mesh/material/`DepositVisual` triggers conversion from its authored Sprite. Later passes update quantity appearance and resize only when the remembered local size changes. The visual shares its target's entity lifetime. [Deposit reconciliation](../../src/deposit_presentation.rs) |
| Nanobot and rock visual attachment | Adding `Nanobot` attaches its presentation child. Adding `RockSurfaceRoot` creates rock mesh children. Entity hierarchy cleanup removes children with their session roots. [Nanobot observer](../../src/nanobot/sprites.rs), [rock initialization](../../src/terrain_presentation.rs) |
| Structure/cargo overlays | Existing target/kind coverage distinguishes absent, valid, changed-kind, and obsolete overlays. Cargo bars appear during extraction/loading or while cargo is present. A kind change replaces the overlay, and target/job loss removes it. [Overlay reconciliation](../../src/structure_overlay.rs) |
| Condition overlays | Membership per target and condition kind creates Maintenance/health bars for support structures and shift-progress bars for active Maintenance Workers. Losing the tracked structure or progress component removes the bar. [Condition overlays](../../src/structure_overlay.rs) |
| Tactical markers | Retained cluster keys and positions match the closest prior marker in each key bucket. Matched markers update, new clusters spawn, and unmatched markers despawn. Visibility derives from zoom. [Marker reconciliation](../../src/tactical_overlay.rs) |
| UI mirrors | Brush selection change detection updates layer highlights. The banner caches its last outcome. Pointer capture, status text, button styling, and zoom visibility are recalculated from current input or world state. [Layer panel](../../src/ui/intent_layer_panel.rs), [banner](../../src/ui/match_banner.rs), [UI pointer gate](../../src/ui/ui_interaction_system.rs) |

Verification covers [deposit presentation](../../tests/behavior/deposit_presentation.rs), [structure overlays](../../tests/behavior/structure_overlay.rs), [combat presentation](../../tests/behavior/combat_presentation.rs), and [offscreen screenshots](../../screenshots/main.rs).

## Presentation transients

Resolved fixed-step combat facts create frame-driven effects. First-frame flags preserve the initial visible pose, and elapsed duration drives recovery and expiry. Presentation state belongs to visual children or dedicated resources.

| Transient | Retained phases |
| --- | --- |
| Nanobot `CombatPose` | Hit aggregation installs or refreshes attack/reaction directions and intensity. First frame holds the initial pose, subsequent frames recover, and expiry restores identity/white and removes the component. |
| `StructureCombatFlash` | A hit saves the neutral color, starts or refreshes a flash, and expiry restores that color and removes the flash. |
| Pulses and decorations | Facts append entries with first-frame and elapsed state. Duration expiry removes entries. Decorations also evict older entries at the configured bound. |
| Nanobot and structure death ghosts | Resolved death creates a detached visual snapshot. Time shrinks/fades the Nanobot or expands/fades the structure ring, then removes the entry. The gameplay victim can already be gone. |
| Cancelled-plan visual | A presentation-only entity retains elapsed ticks and initial scale. It holds briefly, then shrinks/fades and despawns. It has no reservation or collision identity. |

Accepted presentation edits retime active poses, structure flashes, pulses, decorations, and death ghosts by preserving elapsed lifetime fraction. Camera sensitivity updates acceleration, maximum speed, friction, and zoom speed; velocity is clamped to the new maximum while position and zoom remain unchanged. [Effect reconciliation](../../src/nanobot/combat_presentation.rs), [camera reconciliation](../../src/fly_camera.rs), [presentation tuning tests](../../tests/behavior/presentation_tuning.rs).

Combat zoom suppression hides effects while their lifetimes continue advancing. Session reset clears active resource effects and queued facts, and session entity teardown removes visual components and cancellation residue. [Combat effects](../../src/nanobot/combat_presentation.rs), [cancelled-plan effect](../../src/structure_overlay.rs), [combat presentation ADR](../adr/0014-combat-presentation-follows-resolved-combat.md), [presentation tests](../../tests/behavior/combat_presentation.rs)

## Tooling and legacy state

| Owner | State and transitions |
| --- | --- |
| Screenshot `TestDriver` | Explicit `DriverState` advances running callback → requested capture → waiting for readback → resumed callback or exit. Callback/readback limits and failures request cleanup. Full app updates continue while callback delivery is paused. [Harness](../../screenshots/harness.rs) |
| `OffscreenImageExporter` | Optional source and active capture distinguish unprimed, ready, exporting, file-ready, worker-draining, and complete. It retires the exporter entity, waits for workers, moves the file, and clears the active entry. Cleanup and RAII guards remove temporary artifacts and drain outstanding work. [Exporter](../../screenshots/harness.rs) |
| Loaded movement diagnostics | Map membership begins tracking when cargo is positive. Stationary time triggers a reporting latch, movement reports resumption and resets it, and empty cargo or entity loss removes tracking. [Diagnostic observer](../../examples/movement_diagnostics.rs) |
| Local-avoidance fixture | Explicit `Phase` moves through staged approach/retreat/resumption/arrival, with pause gates controlled by the example's input flow. [Fixture](../../examples/local_avoidance.rs) |
| Legacy `AiPlugin` | `AiActionKind` explicitly names actions, while queued requests and action components form a second encoding. The registered scaffold emits Idle requests, drains the queue by priority, and replaces action components. `decision_system` leaves `current_action` at its prior value, so it cannot serve as an authoritative mirror. Gather/Build action systems are empty. [AI scaffold](../../src/ai.rs), [plugin installation](../../src/lib.rs) |

The Python and shell utilities execute command sequences and bounded readiness/shutdown checks. Scenario definitions, CLI parsing, rendering shader calculations, and benchmark timing accumulators are setup, computation, or measurement around these workflows. [Scripts](../../scripts/), [benchmark sources](../../benches/)
