# Top-Down 2D RTS Prototype: Nano Swarm

Nano Swarm is a top-down RTS about steering a swarm of autonomous Nanobots. Paint Gather, Build, Defend, and Corridor zones to direct work across the map. Workers extract minerals and build structures, Haulers carry supplies, and Defenders respond to threats. Each Nanobot chooses work from its swarm’s intent. Both swarms can paint the same cell.

In Standard, eliminate the opposing swarm by destroying its Nanobots and completed structures. A surviving bot or completed structure keeps its swarm in the match. [Gameplay glossary](GLOSSARY.md)

## Running

Run the normal windowed game:

```bash
cargo run
```

Pan with **WASD**, the **arrow keys**, the **middle mouse button** (drag), or the screen edge. Scroll the wheel to zoom around the pointer. Press **Home** to frame the starting base and its deposit again.

Select **Gather**, **Build**, **Defend**, or **Corridor** with **1–4** or the intent buttons. The outline shows the cell under the pointer. Left-drag to paint and right-drag to erase the selected layer. Press **F3** to toggle the FPS counter.

Press **ESC** to pause and open the scenario menu. Choose **Standard** for the opponent match, **Sandbox** for open-ended play without an opponent, or **AI Battle** to watch two automatically controlled swarms and record battle statistics. The choice saves immediately for future launches. **Start selected scenario** begins a fresh match in the current window and resets the camera, even when restarting the same scenario. ESC closes the menu and returns to the current session. **Quit to desktop** exits the game.

Press **F2** to open the live tuning panel. Expand a group and edit its numeric fields, or use Up/Down to step a focused value. Each valid change saves immediately and applies to both swarms. Invalid input or a save failure leaves the active value unchanged and shows an error beside the field. Escape releases field focus; with no field focused, Escape opens the scenario menu. The panel leaves simulation running.

The game loads the complete [balance configuration](config/balance.ron) before creating a scenario. Missing, malformed, or invalid configuration prevents startup. The default path is anchored to the project root; `--tuning-file PATH` selects another file for both loading and saving. Tuning survives scenario restarts and future launches.

Run an accelerated headless AI Battle. It keeps statistics in memory and serves them over the agent socket until you send `shutdown`:

```bash
cargo run --release -- --scenario ai-battle --headless --unbounded --seed 42
```

See [AI Battle](docs/ai-battle.md) for statistics fields, end-of-battle behavior, and comparison guidance.

Run with GPU-backed offscreen rendering and no desktop window:

```bash
cargo run -- --headless
```

Every launch, windowed or headless, opens the agent control socket at `$XDG_RUNTIME_DIR/nano-swarm/control.sock`, so an agent can inspect or control the game you are playing. Startup fails if `XDG_RUNTIME_DIR` is unset or another nano-swarm process holds the socket; give a second instance its own `XDG_RUNTIME_DIR`. nano-swarm supports only Unix. The standard-library client controls a running process:

```bash
python scripts/nano_swarm_control.py state
python scripts/nano_swarm_control.py button intent.defend
python scripts/nano_swarm_control.py paint defend 2 0
python scripts/nano_swarm_control.py camera 1024 256 2
python scripts/nano_swarm_control.py wait --fixed-ticks 60
python scripts/nano_swarm_control.py screenshot
python scripts/nano_swarm_control.py shutdown
```

Run `cargo run -- --help` for runtime options and see [`docs/agents/agent-control.md`](docs/agents/agent-control.md) for the protocol and synchronization contract.
