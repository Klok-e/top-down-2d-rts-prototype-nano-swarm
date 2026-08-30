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
A player-painted spatial region that expresses what kind of work should happen there. Intent zones are the primary command surface for directing the swarm. Intent at a location is binary: painting adds it, repeated painting has no further effect, and erasing removes it.
_Avoid_: Group zone, activity zone, command area

**Swarm Tile**:
A cell containing at least one intent layer owned by a swarm, or a Defend Contest in which that swarm participates. Different owned layers can make the same cell territory for multiple swarms; shared unowned intent does not make it territory.
_Avoid_: Visible intent cell, occupied cell, shared intent cell

**Threat**:
A hostile nanobot or structure physically present on a Swarm Tile. Each Threat attracts at most one pursuit response; danger and proximity prioritize scarce responders, while attacks may target the nearest hostile in range.
_Avoid_: Defend-zone intruder, recent attacker, hostile nanobot only

**Pursuit Halo**:
The one-cell-wide band surrounding Swarm Tiles, including cells that touch only at a corner. It permits an existing defense response to continue briefly outside swarm territory but does not make nearby hostile entities Threats by itself or constrain Defender travel routes.
_Avoid_: Swarm territory, detection range, unlimited pursuit

**Gather Zone**:
An intent zone where nanobots extract resources from available deposits. Each Resource Deposit contributes work once for each eligible swarm regardless of how many of that swarm's painted cells overlap it; paint establishes eligibility, while deposit work determines nanobot demand. Gather intent persists when local resources are depleted; workers leave when no useful work remains, and the zone can reactivate if resources appear later.
_Avoid_: Mining zone, resource zone

**Resource Deposit**:
A map object that contains extractable resources for gather work. It is separate from the resource kind it contains, so a mineral-bearing deposit is still a resource deposit.
_Avoid_: Mineral node, mineral patch, resource pile

**Build Zone**:
An intent zone that marks free base space where automatic construction may place production facilities, sink stockpiles, and similar support structures. Build zones are not direct building placement commands; they constrain where base infrastructure may emerge. Zone area provides placement options but does not itself create construction demand.
_Avoid_: Construction group, builder assignment, manual building placement

**Defend Zone**:
An intent zone that distributes unengaged Defenders at equal density and supports continuous density-driven, procedural roaming within and between its cells. It positions rather than bounds defense or creates population demand: Threats override staging, paint changes rebalance the cohort, and fallback staging uses Swarm Tiles or current cells.
_Avoid_: Fighter group, combat squad, attack zone

**Defend Contest**:
A shared claim created when one swarm paints Defend intent over another swarm's Defend layer. Any living participant Defender physically inside establishes presence; after both sides engage, the sole remaining side captures the layer.
_Avoid_: Territory overlap, attack zone, occupation timer

**Stockpile**:
A local resource buffer automatically created where sustained material flow is needed. Source stockpiles stage gathered resources near deposits; sink stockpiles stage minerals for terminal consumers. Terminal buffers receive minerals only through physical hauler delivery.
_Avoid_: Deposit zone, global storage

**Stockpile Capacity**:
The maximum material a stockpile can hold. It is a local buffer limit, not a global storage cap.
_Avoid_: Stockpile size, global cap

**Source Stockpile**:
A stockpile placed near a resource deposit to receive resources extracted by workers before haulers move them onward.
_Avoid_: Mining depot, deposit storage

**Sink Stockpile**:
A shared stockpile placed in a build zone that stages material between hauler legs: it receives material from source stockpiles and may supply both production facilities and chargers through physical hauler transport. It is not a terminal buffer.
_Avoid_: Global storage, base inventory, production facility hopper

**Logistics Leg**:
One directed hauler movement along the material chain: source stockpile to sink stockpile, or sink stockpile to a terminal. Legs are ranked downstream-first so terminals are fed before buffers are filled.
_Avoid_: Transport step, conveyor segment

**Logistics Reservation**:
A carrying nanobot's temporary claim on source minerals and destination capacity for one resource movement. A reservation prevents competing assignments but never changes mineral custody, location, or quantity.
_Avoid_: Resource transfer, inventory deduction, delivery

**Charge**:
A Defender sustain resource restored by the nearest valid supplied Charger in owned Defend paint. Rotation is capacity-limited and releases any Threat response; unsupported Defenders continue duty while weakening, and recharged Defenders re-enter current allocation without reclaiming prior work.
_Avoid_: Ammo, mana, stamina

**Charger**:
A terminal consumer whose finite local mineral buffer restores Charge for its swarm while it is operational, supplied, and inside owned Defend paint. Unserved low Charge creates nearby capacity plans; a Charger outside owned Defend paint is inactive, while unattended valid Chargers receive no Maintenance and may decay.
_Avoid_: Charge stockpile, instant resupply, resource sink

**Terminal Consumer**:
An end-of-chain structure that only receives material and never serves as a hauler source. Production facilities and chargers are terminals; stockpiles are not, even when a sink stockpile is the source for the next leg.
_Avoid_: Sink, consumer building, final destination

**Opponent Swarm**:
A non-player swarm governed by the same intent, production, logistics, maintenance, and Charge rules as the player swarm. An authored opponent may use a deterministic intent controller that advances its Defend intent toward a target while leaving nanobot allocation, production, logistics, Maintenance, Charge, and combat to the shared simulation.
_Avoid_: Enemy AI faction, scripted attackers

**Production Collapse**:
A terminal win or loss condition where unmet workload remains but a swarm has neither operational production nor a complete physical recovery path. The first detected match result remains latched even if later simulation state changes. Recovery requires usable construction space or an owned production plan, appropriate Worker/Hauler capability, and reachable material; surviving crew alone is insufficient.
_Avoid_: Population wipeout, king unit death, crew-count proxy

**Automatic Construction**:
The swarm creates needed structures from demand pressure rather than direct player placement. Production facilities, stockpiles, chargers, and similar support structures emerge inside or near matching intent paint when existing capacity is too busy for current intent. Painting a Build Zone alone does not create a structure; there must be active demand for the resulting support structure.
_Avoid_: Manual building placement, blueprint palette

**Minimum Category Activation**:
When an intent category has valid work and available eligible nanobots, the swarm should keep at least some work active in that category before letting normal scoring optimize the rest. This makes newly drawn valid intent visibly receive workers quickly without requiring direct unit control.
_Avoid_: Manual assignment, hard quota, perfect allocation

**Planned Structure**:
A stable, owner-scoped commitment to build one support structure at one location. It persists when the demand that triggered it recedes and resolves only by completion or destruction.
_Avoid_: Blueprint, ghost building, construction order

**Building Footprint**:
The world area visibly occupied by a planned or completed support structure. Planned and completed forms reserve the same kind-specific footprint, which cannot overlap other structures or resource deposits; nanobots do not block it.
_Avoid_: Generic sprite size, unit collision

**Maintenance**:
Ongoing worker time required to prevent structure collapse. A structure remains fully functional while any health remains and is destroyed at zero; overexpansion or cut-off worker access creates collapse risk rather than partial shutdown.
_Avoid_: Permanent buildings, fire-and-forget construction

**Overlapping Intent**:
Multiple intent zones may cover the same space. Overlap means several kinds of work are valid there; autonomous allocation decides which nanobots respond without a player-set task priority.
_Avoid_: Exclusive zones, zone ownership

**Intent Allocation**:
The moment-to-moment act of steering the swarm by adjusting intent zone placement and size, plus Production Priority. Players do not prioritize individual tasks; autonomous allocation weighs useful work, distance, type fit, crowding, and commitments. This is the primary player skill, not micro-managing individual nanobots.
_Avoid_: Unit micro, direct control

**Soft Work Slot**:
A limited amount of useful work available at a resource, build site, or threat. Extra nanobots are less useful and may wait, crowd, or choose other work, but are not strictly forbidden from being nearby.
_Avoid_: Hard assignment slot, infinite work stack

**Dumb Autonomy**:
Nanobots are aware of player-painted intent globally, but execute it through simple scoring rather than optimal assignment. Their limitations create player-facing pressure through congestion, travel time, carrying capacity, production ordering, and over/under-painting rather than through failing to notice commands.
_Avoid_: Perfect allocator, smart commander AI

**Global Intent Awareness**:
All valid player-painted intent can eventually attract eligible nanobots, even when no nanobot currently searches it directly. Awareness may be mediated through regional demand and bounded local decisions rather than every nanobot evaluating every intent cell. Response is weighted by useful work, distance, type fit, local crowding, and current commitments, so nearby or idle nanobots usually respond first.
_Avoid_: Local-only awareness, hidden command radius

**Commitment**:
A nanobot's tendency to finish its current short task before reconsidering player intent. Carrying nanobots complete a valid delivery, reroute when a destination becomes invalid, or physically return cargo to a compatible stockpile when no destination can receive it.
_Avoid_: Instant retargeting, hard lock-in

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
Resources move physically through nanobots carrying them. Minerals remain swarm-owned while in cargo, so transport changes custody and location but not total quantity; consumption or destruction of a loaded nanobot removes minerals from the swarm.
_Avoid_: Global stockpile, teleporting resources

**Logistics Corridor**:
A player-painted movement intent for haulers that encourages resource transport along a path between stockpiles, facilities, chargers, or other resource needs. Owned Corridor cells apply a fixed route bias; their shape defines the preferred path, while unpainted or enemy Corridor cells give no benefit. Corridors do not create resource tasks by themselves and are special hauler guidance, not general direct movement commands.
_Avoid_: Road, waypoint chain, manual route

**Defender**:
A nanobot type that stages and roams in Defend Zones, responds to Threats throughout its swarm's Swarm Tiles, and may briefly pursue them through the Pursuit Halo.
_Avoid_: Fighter, soldier, combat unit

**Production Priority**:
A player-set relative weighting that orders unmet Worker, Hauler, and Defender demand. It does not create demand or promise a population mix. Production balances shortage size against priority, while a zero-priority type remains eligible when its work is required.
_Avoid_: Build queue, manual unit training

**Population Demand**:
The per-Nanobot-Type capacity justified by actionable workload: gathering, construction, and maintenance require Workers, while physical transport requires Haulers. Defender demand is the greater of half the swarm's unique Swarm Tile count rounded up and its active Threat count; Defend Zones only position that capacity. Existing and in-production nanobots of one type cannot satisfy another type's demand. Excess nanobots remain in the swarm when demand falls. A pending Production Facility may increase Worker demand but is never evidence for committing another Production Facility.
_Avoid_: Population cap, unit quota

**Production Pressure**:
Consecutive unmet Population Demand of any type while every operational Production Facility remains busy. Production Priority, including a zero weight, never suppresses pressure from required work. Pressure also accumulates when no Production Facility is operational, while resolved demand or idle operational capacity clears it. Brief demand spikes and a facility's internal cycle progress do not establish Production Pressure.
_Avoid_: Build-zone size, instantaneous deficit

**Production Facility**:
A terminal support structure that consumes delivered resources and automatically fills typed Population Demand in Production Priority order. Production Pressure may create one unfinished expansion commitment per swarm; capacity is reassessed after that commitment completes or is lost.
_Avoid_: Barracks, factory queue, manual spawner
