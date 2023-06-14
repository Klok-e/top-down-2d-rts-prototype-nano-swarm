# Top-Down 2D RTS Prototype: Nano Swarm

## Description

Nano Swarm is a prototype of a real-time strategy (RTS) game where players command massive swarms of nanobots in epic scale battles. The game leverages the Entity-Component-System (ECS) architecture for efficient performance, even with hundreds of thousands of individual units.

## Core Gameplay

1. **Dynamic Grouping**: Players manage groups of nanobots that can be easily split, merged, or reassigned for flexible strategic decision-making.

2. **Role-Based Behavior**: Groups of nanobots have specific roles (gatherers, builders, fighters, etc.) that dictate their behavior and the commands they respond to.

3. **Activity Zones**: Players assign specific activity zones to nanobot groups. These zones guide the group's behavior and interactions.

4. **Task Interactions**: When two activity zones overlap, nanobots perform combined tasks, leading to interesting strategic considerations.

5. **Automated Resource Management**: Gatherer groups automatically collect and transport resources between their designated zones, reducing micromanagement.

6. **Adaptive Combat System**: Fighter groups can be directed to specific areas where they spread out and engage enemies, simplifying large-scale combat operations.

7. **Persistent Evolution**: The game state continually changes, requiring players to adapt their strategies effectively to succeed.
