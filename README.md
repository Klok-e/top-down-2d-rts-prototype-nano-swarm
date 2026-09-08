# Top-Down 2D RTS Prototype: Nano Swarm

## Introduction

Nano Swarm is a real-time strategy game prototype developed in a top-down 2D environment. The game utilizes the Entity-Component-System (ECS) architecture to effectively manage hundreds of thousands of nanobots, each represented as individual units.

## Core Gameplay

The core gameplay of Nano Swarm is based on a set of key mechanics:

1. **Dynamic Grouping**: Players can create and manage groups of nanobots dynamically, providing flexibility in response to changing battlefield conditions. A group can be interacted with by selecting any individual bot from the group.

2. **Role-Based Behavior**: Each nanobot group can be assigned specific roles such as gatherers, builders, or fighters. The assigned role determines the behavior of the group and the commands they respond to.

3. **Activity Zones**: Players assign specific zones of activity to each nanobot group. These zones guide the behavior of nanobots based on their role.

4. **Task Interactions**: Overlapping zones of different groups enable bots to perform combined tasks, adding a layer of strategic depth. For example, a gatherer group's zone overlapping with a building zone results in efficient resource transportation.

5. **Automated Resource Management**: Gatherer groups autonomously manage resources within their assigned zones, reducing the need for constant player input for resource management.

6. **Adaptive Combat System**: Players can direct fighter groups to specific zones where they spread out and engage enemy bots, simplifying control in large-scale battles.

7. **Persistent Evolution**: The game state continually changes due to various factors, such as enemy activity, resource availability, or the need for base expansion. This requires players to adapt their strategies continually to succeed.

## Objective

The primary objective of the game is to manage resources effectively, build a resilient base, command nanobot groups strategically, and overcome opponent forces to gain control over the map.

## Running

Run the normal windowed game:

```bash
cargo run
```

Press **ESC** to pause and open the scenario menu. Choose **Standard** for the opponent match, **Sandbox** for open-ended play without an opponent, or **AI Battle** to watch two automatically controlled swarms and record battle statistics. The choice saves immediately for future launches. **Start selected scenario** begins a fresh match in the current window and resets the camera, even when restarting the same scenario. ESC closes the menu and returns to the current session. **Quit to desktop** exits the game.

Run an accelerated headless AI Battle with saved statistics:

```bash
cargo run --release -- --scenario ai-battle --headless --seed 42 --output-root target/battle-runs
```

See [AI Battle](docs/ai-battle.md) for recording fields, termination behavior, and comparison guidance.

Run with GPU-backed offscreen rendering and no desktop window:

```bash
cargo run -- --headless
```

Enable the opt-in Unix agent control socket in either presentation mode:

```bash
cargo run -- --agent-socket
cargo run -- --headless --agent-socket --width 1280 --height 720
```

`--agent-socket` listens at `$XDG_RUNTIME_DIR/nano-swarm/control.sock`. The standard-library client controls a running process:

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
