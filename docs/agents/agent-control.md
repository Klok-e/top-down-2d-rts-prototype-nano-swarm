# Agent Control

The agent control interface drives a real nano-swarm process without OS input automation. It is opt-in and works with either the normal window or GPU-backed offscreen presentation.

Agents may extend the agent-control interface as they see fit.

Automated tests and agent verification must never create an OS/compositor window. Use minimal ECS apps or offscreen rendering; run real agent playtests with `--headless --agent-socket`.

## Launch

```bash
cargo run -- --agent-socket
cargo run -- --headless --agent-socket
cargo run -- --headless --agent-socket --width 1280 --height 720
```

`--agent-socket` requires `XDG_RUNTIME_DIR` and listens on:

```txt
$XDG_RUNTIME_DIR/nano-swarm/control.sock
```

Headless mode disables Winit and creates no desktop window. Rendering, UI layout, fixed simulation, and GPU screenshots remain active. The runner handles `AppExit` and Ctrl-C; its default pacing is 60 application frames per second. AI Battle selects accelerated headless execution, advancing one 60 Hz simulation tick per application update without real-time pacing. See [AI Battle](../ai-battle.md) for launch options and saved statistics.

Headless width and height are each capped at 8,192 pixels, with a total budget of 16,777,216 pixels, so invalid CLI input fails before allocating the render image.

## Client

The client uses only Python's standard library:

```bash
python scripts/nano_swarm_control.py hello
python scripts/nano_swarm_control.py state
python scripts/nano_swarm_control.py state --cell-offset 10000 --cell-limit 10000 --map-revision 42
python scripts/nano_swarm_control.py button intent.defend
python scripts/nano_swarm_control.py menu
python scripts/nano_swarm_control.py button menu.sandbox
python scripts/nano_swarm_control.py button menu.start
python scripts/nano_swarm_control.py select defend
python scripts/nano_swarm_control.py paint defend 2 0
python scripts/nano_swarm_control.py erase defend 2 0
python scripts/nano_swarm_control.py camera 1024 256 2
python scripts/nano_swarm_control.py pan 128 -64
python scripts/nano_swarm_control.py wait --fixed-ticks 60
python scripts/nano_swarm_control.py screenshot --name assault
python scripts/nano_swarm_control.py shutdown
```

Use `--socket PATH` to target a non-default socket and `--timeout SECONDS` to override the 320-second client timeout.

## Wire Format

The socket accepts one client at a time. Requests are processed sequentially as newline-delimited JSON. One line may contain at most 65,536 bytes, excluding its newline, and must complete within 30 seconds.

```json
{"id": 7, "method": "map.apply", "params": {"action": "paint", "intent": "defend", "x": 2, "y": 0}}
```

Request IDs may be unsigned integers or strings. Successful responses have this shape:

```json
{"id": 7, "ok": true, "frame": 120, "fixed_tick": 121, "result": {"changed": true}}
```

Errors use the same completion clocks:

```json
{"id": 7, "ok": false, "frame": 120, "fixed_tick": 121, "error": {"code": "match_finished", "message": "the match is already complete"}}
```

The response ID is `null` only when an invalid or oversized request does not contain a recoverable ID.

## Synchronization

`frame` is the number of main-world frames completed in `Last`, immediately before render submission. `fixed_tick` is the number of completed 60 Hz simulation ticks. Immediate command responses contain the last completed clocks observed when the command enters `PreUpdate`; the command contributes to the following frame. Use `frame.wait` when a command must complete a subsequent frame before inspection. A subsequent command on the sequential socket enters on a later frame.

`frame.wait` accepts `frames`, `fixed_ticks`, or both. It responds only after both requested deltas have completed. Each delta is limited to 18,000, or five minutes at 60 Hz. The server adds ten seconds of response-deadline grace for scheduling overhead. Waiting does not pause rendering or simulation.

```json
{"id": 8, "method": "frame.wait", "params": {"frames": 30, "fixed_ticks": 60}}
```

Screenshot capture is asynchronous. A request received before any render submission is deferred to the first submitted frame so an immediate startup capture cannot read the offscreen target's unrendered clear state. Simulation continues while GPU readback is pending. Its response is sent only after PNG encoding, file creation, and owner-only permissions complete. `capture_frame` and `capture_fixed_tick` are stamped immediately before the target frame is submitted to the renderer; the response envelope clocks identify later readback completion when applicable.

## Methods

| Method | Parameters | Result |
| --- | --- | --- |
| `session.hello` | none | Protocol version and supported methods |
| `state.get` | optional `cell_offset`, optional `cell_limit`, optional `map_revision` | Sparse game-state snapshot page |
| `button.press` | `button` | Activates a stable real UI button |
| `menu.toggle` | none | Opens or closes the scenario menu, like Escape |
| `intent.select` | `intent` | Selects an intent directly |
| `map.apply` | `action`, `intent`, `x`, `y` | Whether intent state changed |
| `camera.set` | `x`, `y`, optional `zoom` | Applied camera view |
| `camera.pan` | `dx`, `dy` | Applied camera view |
| `frame.wait` | optional `frames`, optional `fixed_ticks` | Actual elapsed clocks |
| `screenshot.capture` | optional `name` | Absolute path, dimensions, and capture clocks |
| `process.shutdown` | none | Requests a clean successful `AppExit` |

Valid intents are `gather`, `build`, `defend`, and `corridor`. Stable button IDs are `intent.gather`, `intent.build`, `intent.defend`, and `intent.corridor`.

Player-action commands are rejected after Victory, Defeat, or Draw. State, camera, screenshot, wait, hello, and shutdown remain available for terminal-state inspection.

## State Snapshot

`session.hello` reports protocol version 4.

`state.get` returns:

- Selected intent.
- Map dimensions, sparse active cells, per-layer swarm owners.
- Main-camera position and zoom.
- Match outcome (`in_progress`, `victory`, `defeat`, or `draw`) and `player_eliminated` / `opponent_eliminated` flags. AI Battle uses `swarm_0_wins` / `swarm_1_wins` instead of `victory` / `defeat`; its elimination flags refer to Swarm 0 / Swarm 1 respectively.
- Per-swarm `eliminated` flag, population, demand, aggregate health, centroid, minerals, and facility counts.

Empty map cells are omitted. Active cells use deterministic row-major ordering. Each response includes at most 10,000 active cells with their independently owned intent layers, `active_cell_total`, `next_cell_offset`, and `map_revision`. Pass both `next_cell_offset` and the unchanged `map_revision` into the next `state.get` call until the offset is `null`. Page zero contains the complete non-map snapshot; continuation pages contain only `map`, preventing live simulation changes from mixing newer swarm or match data into that snapshot. If the map changes between pages, the server returns `stale_state_page`; restart from offset zero. Each layer entry has an intent kind and a non-null owner ID: `0` is the player and positive IDs are opponents. The same kind can appear more than once in a cell, once per owning swarm; consume all entries. Entries are ordered by intent kind, then owner ID. There is no `defend_contests` field.

## Player Equivalence

Socket map edits call the same semantic path as mouse painting:

- Painting any kind adds the player's own intent, including over enemy paint.
- Erasing removes only the player's selected kind and preserves all enemy orders.
- Out-of-bounds edits and post-match player actions return errors.

`button.press` sets the real button's `Interaction::Pressed`, lets the existing UI click system process it, and releases it on the next frame. Camera commands preserve camera depth, synchronize projection and zoom state, and clear keyboard movement velocity.

## Screenshots

Windowed capture targets the primary window. Headless capture targets the main camera's offscreen image. No compositor capture or fallback exists.

Files are written below:

```txt
$XDG_RUNTIME_DIR/nano-swarm/screenshots/
```

Generated names are process- and sequence-scoped. An optional name may contain only ASCII letters, digits, `-`, and `_`. The Unix transport processes one request at a time, so external screenshot requests are serialized. The core rejects another capture if one is already pending through a non-serial transport.

## Security And Cleanup

The server creates `$XDG_RUNTIME_DIR/nano-swarm` with mode `0700` and the socket and lifecycle lock with mode `0600`. A pre-existing socket parent must already be a real directory owned by the current user with no group or world access; the server rejects it unchanged otherwise. It retains an exclusive lifecycle lock, refuses to replace non-socket paths or an active server, and removes a stale socket only after a failed socket probe while holding that lock. All Bevy world access remains on the main thread behind a bounded request queue.

The socket worker polls a nonblocking listener and performs bounded blocking client I/O with timeouts. It detects fully disconnected clients without rejecting request-write half-closes, cancels pending waits or captures by request instance, and enforces a five-minute response deadline plus ten seconds of grace. `process.shutdown`, Ctrl-C, and normal app teardown stop the worker, join it, and remove only the socket inode created by that process.

## Troubleshooting

`XDG_RUNTIME_DIR is required`: launch from a desktop/session environment that defines it, or set it to a private runtime directory owned by the current user.

`another nano-swarm control server is active`: use the existing process or shut it down before starting another controlled process.

`control response did not complete`: verify the game process is still running and increase the client `--timeout` for long fixed-tick waits or GPU capture.

`screenshot_failed`: inspect GPU adapter diagnostics in the game log. Headless mode requires a working Bevy/wgpu adapter but never falls back to a desktop window.

`spectator_only`: AI Battle rejects `map.apply` painting and erasing because both swarms own their control. Use camera, state, screenshots, and menu controls to observe the run. For headless benchmark execution and saved results, read [AI Battle](../ai-battle.md).

`match_finished`: use `state`, `camera`, or `screenshot` to inspect the terminal state, or open the menu and start a fresh scenario.

Menu buttons are `menu.start`, `menu.standard`, `menu.sandbox`, `menu.ai_battle`, and `menu.quit`. Use `menu.toggle` to open or close the menu through the same transition as Escape. Menu actions use the real UI buttons and remain available after a match finishes; buttons inside the menu require it to be open. `menu.start` immediately replaces the current match with the selected scenario. While the menu is open, world input commands (including camera commands) return `menu_open`. Use frame waits rather than fixed-tick waits while paused. `state.get` includes `scenario` with `current`, `next_launch`, `save_error`, and `menu_open`. The runtime reads the persisted preference from `$XDG_CONFIG_HOME/nano-swarm/scenario.json`, or `$HOME/.config/nano-swarm/scenario.json` when XDG_CONFIG_HOME is unavailable; automated runtime checks should use an isolated configuration directory.
