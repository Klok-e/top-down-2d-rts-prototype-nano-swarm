# Nano Swarm

Nano Swarm is a top-down RTS prototype about steering a population of autonomous nanobots through spatial intent rather than managing fixed unit groups.

## Language

**Swarm**:
The full player-controlled population of nanobots. It has no fixed subgroups; player intent is expressed through zones and tasks that eligible nanobots self-assign to.
_Avoid_: Group, squad, unit group

**Nanobot**:
An individual autonomous agent within the swarm. It may choose tasks from player intent, but is not a persistent command target.
_Avoid_: Unit, soldier, worker

**Intent Zone**:
A player-painted spatial region that expresses what kind of work should happen there. Intent zones are the primary command surface for directing the swarm.
_Avoid_: Group zone, activity zone, command area

**Gather Zone**:
An intent zone where nanobots extract resources from available deposits. Gather intent persists when local resources are depleted; workers leave when no useful work remains, and the zone can reactivate if resources appear later.
_Avoid_: Mining zone, resource zone

**Resource Deposit**:
A map object that contains extractable resources for gather work. It is separate from the resource kind it contains, so a mineral-bearing deposit is still a resource deposit.
_Avoid_: Mineral node, mineral patch, resource pile

**Build Zone**:
An intent zone that marks free base space where automatic construction may place production facilities, sink stockpiles, and similar support structures. Build zones are not direct building placement commands; they constrain where base infrastructure may emerge.
_Avoid_: Construction group, builder assignment, manual building placement

**Defend Zone**:
An intent zone where nanobots hold and protect an area. Painting defend intent into enemy territory functions as an attack or advance order; no separate attack zone is needed initially. Defend zones include chargers that resupply defenders, making cut-off or surrounded defenders weaker over time.
_Avoid_: Fighter group, combat squad, attack zone

**Stockpile**:
A local resource buffer automatically created where sustained material flow is needed. Source stockpiles stage gathered resources near deposits; sink stockpiles in build zones feed production facilities, chargers, and future infrastructure.
_Avoid_: Deposit zone, global storage

**Source Stockpile**:
A stockpile placed near a resource deposit to receive resources extracted by workers before haulers move them onward.
_Avoid_: Mining depot, deposit storage

**Sink Stockpile**:
A stockpile placed in a build zone or base area to feed production facilities, chargers, construction, and other infrastructure needs.
_Avoid_: Global storage, base inventory

**Charger**:
A local support structure automatically created where defenders need resupply. Defender effectiveness depends on regularly visiting chargers; defenders that go too long without charging lose health and attack/defense strength. Charger creation responds to defend-zone load and existing charger busyness, and chargers require logistics support so isolated defenses degrade when haulers cannot reach them.
_Avoid_: Ammo crate, healing station

**Charge**:
A defender sustain resource restored by visiting chargers. Only defenders use charge. Low charge weakens defender attack and defense, then causes health loss if ignored too long. Defenders automatically rotate to working chargers when charge runs low; fresh defenders can replace them at the front.
_Avoid_: Ammo, mana, stamina

**Opponent Swarm**:
A non-player swarm governed by the same intent, production, logistics, maintenance, and charge rules as the player swarm. Early opponents use prepainted bases and fixed production ratios instead of active AI.
_Avoid_: Enemy AI faction, scripted attackers

**Production Collapse**:
A win or loss condition where a swarm loses the ability to recover because it has no working production capacity and too few remaining nanobots to rebuild it.
_Avoid_: Population wipeout, king unit death

**Automatic Construction**:
The swarm creates needed structures from demand pressure rather than direct player placement. Production facilities, stockpiles, chargers, and similar support structures emerge inside or near matching intent paint when existing capacity is too busy for current intent.
_Avoid_: Manual building placement, blueprint palette

**Planned Structure**:
A not-yet-built support structure selected by automatic construction and waiting for worker time. Planned structures have a kind, owner, location, and build work remaining before they become physical structures.
_Avoid_: Blueprint, ghost building, construction order

**Maintenance**:
Ongoing worker time required to keep structures functional. All structures degrade when not maintained, so overexpansion or cut-off worker access can cause infrastructure to weaken or collapse.
_Avoid_: Permanent buildings, fire-and-forget construction

**Overlapping Intent**:
Multiple intent zones may cover the same space. Overlap means several kinds of work are valid there; allocation and priority decide which nanobots respond.
_Avoid_: Exclusive zones, zone ownership

**Intent Allocation**:
The moment-to-moment act of steering the swarm by adjusting intent zone size, paint strength, and production ratio. This is the primary player skill, not micro-managing individual nanobots.
_Avoid_: Unit micro, direct control

**Soft Work Slot**:
A limited amount of useful work available at a resource, build site, or threat. Extra nanobots are less useful and may wait, crowd, or choose other work, but are not strictly forbidden from being nearby.
_Avoid_: Hard assignment slot, infinite work stack

**Dumb Autonomy**:
Nanobots are aware of player-painted intent globally, but execute it through simple scoring rather than optimal assignment. Their limitations create player-facing pressure through congestion, travel time, carrying capacity, imperfect ratios, and over/under-painting rather than through failing to notice commands.
_Avoid_: Perfect allocator, smart commander AI

**Global Intent Awareness**:
All eligible nanobots can consider all player-painted intent zones. Response is weighted by paint strength, need, distance, type fit, and current commitments, so nearby or idle nanobots usually respond first.
_Avoid_: Local-only awareness, hidden command radius

**Commitment**:
A nanobot's tendency to finish its current short task before reconsidering player intent. Idle nanobots respond immediately, carrying nanobots usually finish delivery, and active workers usually finish a short work chunk before reassessing.
_Avoid_: Instant retargeting, hard lock-in

**Paint Strength**:
The intensity of a painted intent zone at a location. Higher paint strength attracts more eligible nanobots and can override weak commitments, but does not make work happen faster by itself. Overpainting can overcommit the swarm, causing congestion, waiting, or starvation elsewhere. Repeated painting increases strength; erasing reduces or removes it. Player-painted intent persists until changed.
_Avoid_: Priority slider, command level

**Nanobot Type**:
A specialization of nanobot with different capabilities or efficiency. The player does not assign individual nanobots to types manually.
_Avoid_: Class, role, group role

**Worker**:
A nanobot type that performs direct work at resource deposits and construction sites, and can carry small resource amounts when needed.
_Avoid_: Harvester, builder, gatherer

**Hauler**:
A nanobot type specialized for transporting resources between places where resources are produced, needed, stored, or processed. Haulers carry much more than workers.
_Avoid_: Carrier, transporter

**Resource Logistics**:
Resources move physically through nanobots carrying them. Workers can move small amounts; haulers are primary transport capacity.
_Avoid_: Global stockpile, teleporting resources

**Logistics Corridor**:
A player-painted movement intent for haulers that encourages resource transport along a path between stockpiles, facilities, chargers, or other resource needs. Corridors bias hauler path choice but do not create resource tasks by themselves; haulers still choose jobs from source and sink need. Corridors are special hauler guidance, not general direct movement commands.
_Avoid_: Road, waypoint chain, manual route

**Defender**:
A nanobot type that protects swarm assets and fights threats.
_Avoid_: Fighter, soldier, combat unit

**Production Ratio**:
A player-set target mix of nanobot types. Production automatically adjusts over time to move the swarm toward this mix. When producing, the swarm prioritizes the type with the largest deficit from the target ratio, skipping blocked types temporarily if their requirements cannot be met.
_Avoid_: Build queue, manual unit training

**Production Facility**:
A player structure automatically created when existing production facilities are too busy for the swarm's production needs. It consumes delivered resources and automatically produces nanobots toward the production ratio. Early nanobot types may share production cost and time; later designs may differentiate costs or requirements.
_Avoid_: Barracks, factory queue, manual spawner

## Agent Workflow Language

**AFK Run**:
A user-started automation run that owns an issue from implementation through verification without requiring main-chat intervention. While an AFK run is active, the main chat observes, reports, or stops it but does not inspect, edit, test, or continue its work unless explicitly instructed.
_Avoid_: Background helper, partial delegation, assistant-side follow-up

**AFK Live Panel**:
A compact progress view for an active AFK run, showing current issue, phase, active agent, recent activity, and where to inspect the full transcript. It is for monitoring, not for transferring work back to the main chat.
_Avoid_: Full transcript dump, main-agent handoff, hidden background job

**AFK Watch**:
An AFK-specific detailed progress view that follows the AFK run's phase story and recent agent activity. It complements the general `/agents` subagent manager, which remains the raw debug view for all subagents.
_Avoid_: Agents replacement, transcript-only view
