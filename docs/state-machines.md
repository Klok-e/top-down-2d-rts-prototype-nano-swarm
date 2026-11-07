# State machines

This inventory describes behavior retained across fixed simulation ticks, rendered frames, requests, and sessions. An implicit state machine encodes its phases through component presence, optional values, counters, flags, or collection membership. Several workflows combine those encodings with an explicit enum for part of the lifecycle.

Each entry identifies the state owner, transitions and guards, interruption and reset behavior, and verification sources. State names describe the implementation and may span several systems. Source links point to owning files, with symbols named in the entries so that line-number changes preserve the navigation.

## Inventory

| Area | Retained workflows |
| --- | --- |
| [Economy and construction](state-machines/economy.md) | Worker gathering and delivery, Hauler logistics and reservations, construction claims, Planned Structures and clearing validation, cancellation exclusions, Maintenance shifts, structure condition, production cycles and Production Pressure, Charge rotation and damage pulses, death settlement, live economy reconciliation, legacy construction |
| [Movement and allocation](state-machines/movement-and-allocation.md) | Exterior work admission, route following and replanning, congestion recovery, traffic yielding, evacuation, idle departure and spread, Regional Leases, acquisition cadence, fairness aging, navigation requests and chunk builds, projection and query caches, Defender response and staging, attack cooldown, live autonomy reconciliation |
| [Runtime and presentation](state-machines/runtime-and-presentation.md) | Headless runner, menu and pause, live tuning publication and editor focus, Scenario selection and replacement, Match Outcome, Strategic Controller planning and cleanup, battle recording, agent socket and request lifecycles, pending waits and screenshots, synthetic button presses, intent render propagation, presentation reconciliation and effects, screenshot harness, diagnostic tracking, legacy AI actions |

## Shared execution and ownership

Gameplay runs in fixed schedules. Navigation refreshes and advances its shared budget in `FixedFirst`. Movement, territory and Threat handling, combat, and Maintenance have ordered simulation phases. Regional projection and invalidation precede acquisition, and Hauler work follows acquisition. Nanobot death settlement and Swarm Elimination run after the tick's effects. Presentation consumes the resulting state in frame schedules. Plugin ordering is part of any transition whose producer and consumer live in different systems. [Simulation wiring](../src/nanobot.rs), [allocation ordering](../src/nanobot/allocation/runtime.rs), [Hauler ordering](../src/nanobot/haul.rs), [navigation ordering](../src/navigation_runtime.rs)

Nanobots retain cargo independently of task leases and endpoint lifetime. Reservations claim future source quantity and destination capacity. Transfers change physical custody, and cancellation releases claims. The economy entries describe the exact cleanup for empty jobs and the delivery obligations of loaded Nanobots. [Cargo and reservations](../src/nanobot/cargo.rs)

Session replacement removes `SessionEntity` roots and their children, resets session resources, clears combat facts and transients, and increments `SessionGeneration`. Generation-aware local maps discard state on their next use. Agent transport and its frame/fixed-tick clocks remain process-owned. Every retained component, local map, resource, or pending operation belongs to one of those lifetimes. [Session replacement](../src/session_lifecycle.rs), [Nanobot resets](../src/nanobot.rs), [agent clocks](../src/agent_control.rs)

## Boundaries of the inventory

The entries include independent timers and cache protocols when remembered history changes a later decision. Inner material pulses and animation timers are grouped with their owning jobs or effects. Test tooling with retained phases is documented alongside runtime machinery.

These related representations have explicit or derived state.

| Representation | Meaning and ownership |
| --- | --- |
| `Commitment` | Explicit `Idle`, `Carrying`, and `Working` values used by allocation and scoring. The detailed job phase lives in the economy components. [Autonomy](../src/nanobot/autonomy.rs) |
| `ApproachPhase`, `ClearingValidation`, navigation result enums | Explicit substates or query results inside the movement and structure workflows. Their surrounding requests, claims, and component lifetimes are covered in the entries. [Work approach](../src/nanobot/work_navigation.rs), [clearing](../src/nanobot/structure_lifecycle.rs), [navigation results](../src/navigation/budget.rs) |
| Intent cell ownership | Binary presence for each swarm/kind/cell. Paint sets a bit, repeated paint is idempotent, and erase clears only that owner's selected kind. Revision and dirty sets propagate actual changes. [Intent data](../src/intent.rs) |
| `BrushSelection` and button `Interaction` | Explicit selection and input values. Menu suppression and synthetic next-frame release supply the surrounding retained behavior. [Brush selection](../src/intent.rs), [agent input](../src/agent_control.rs) |
| Population Demand, territory and elimination snapshots | Recomputed from current intent, entities, and workload. `SwarmEliminationState` can change after a terminal `MatchOutcome`, whose first result stays latched. [Population](../src/nanobot/population.rs), [territory](../src/nanobot/allocation/territory.rs), [elimination](../src/nanobot/elimination.rs) |
| Charger eligibility, resource availability, buffer capacity, HUD status, overlay visibility | Predicates or presentations derived from current inputs. Their changes drive transitions in owning workflows. [Charge](../src/nanobot/charge.rs), [resources](../src/resources.rs), [HUD](../src/ui/status_panel.rs) |
| Camera velocity and zoom | Continuous integration, clamping, and derived rendering state. Menu input suppression and explicit camera reset are covered with the session lifecycle. [Camera movement](../src/fly_camera/camera_move.rs), [zoom](../src/fly_camera/camera_zoom.rs) |
| ID/sequence counters, totals, scratch collections | Identity, accumulation, or temporary working storage. Their owning workflows document any counter that gates a transition. [Battle counters](../src/battle_statistics.rs), [agent sequences](../src/agent_control.rs) |

## Domain decisions

The glossary supplies domain vocabulary, and accepted ADRs supply intended contracts. Entries call out implementation differences beside the affected workflow so that a descriptive inventory does not silently redefine those contracts. [Glossary](../GLOSSARY.md), [ADRs](adr/)
