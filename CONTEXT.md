# Nano Swarm

Nano Swarm is a top-down RTS prototype about steering a population of autonomous nanobots through spatial intent rather than managing fixed unit groups.

## Language

**Scenario**:
A complete starting setup defining terrain, Resource Deposits, starting swarms, controller behavior, and match outcome rules. Standard pairs the player with an advancing Opponent Swarm, Sandbox omits the opponent and match outcomes, and AI Battle pairs two automatically controlled swarms.
_Avoid_: Saved game, map only

**AI Battle**:
A spectator scenario in which two automatically controlled swarms battle from balanced starts on the Standard map or controlled experimental layouts, exercising shared nanobot autonomy without player-painted intervention. The first Swarm Elimination ends the battle and its statistics collection, even if the simulation continues afterward.
_Avoid_: Adaptive strategy AI, player-versus-AI match

**Swarm**:
The full population of nanobots belonging to one side, steered by a player or an automatic controller. It has no fixed subgroups; intent is expressed through zones and tasks that eligible nanobots self-assign to.
_Avoid_: Group, squad, unit group

**Strategic Controller**:
An automatic controller with full knowledge of the current game state that adapts its own swarm's Intent Allocation to maintain an economy, attack, and respond to changing conditions. Nanobots execute its intent through the same autonomy and gameplay rules as the player swarm.
_Avoid_: Nanobot autonomy, scripted assault, direct unit control

**Intent Plan**:
A Strategic Controller's coordinated arrangement of Gather, Build, Defend, and Corridor intent across the map. It expresses intended economic, support, and combat work without assigning individual Nanobots.
_Avoid_: Build order, squad orders

**Combat Damage Credit**:
Hostile combat HP removed from Nanobots and completed structures that counts toward AI Battle strategy evaluation, capped at one maximum-health bar per enemy lifetime across all attackers. Gross damage remains observable, but repairing an enemy cannot renew its credit allowance.
_Avoid_: Damage per second, kill score, game winner

**Nanobot**:
An individual autonomous agent within the swarm. It may choose tasks from player intent, but is not a persistent command target.
_Avoid_: Unit, soldier, worker

**Intent Zone**:
A swarm-owned spatial region that expresses what kind of work should happen there. Intent is binary per swarm, kind, and location: painting adds only that swarm's orders, erasing removes only its orders, and combat never captures or erases them; unowned paint does not exist.
_Avoid_: Group zone, activity zone, command area

**Swarm Tile**:
A cell containing at least one intent layer owned by a swarm. Independent paint of the same or different kinds can make the cell territory for multiple swarms, counting once for each swarm.
_Avoid_: Visible intent cell, occupied cell, shared intent cell

**Threat**:
A hostile nanobot or structure physically present on a Swarm Tile. Each Threat attracts at most one pursuit response; danger and proximity prioritize scarce responders, while attacks may target the nearest hostile in range.
_Avoid_: Defend-zone intruder, recent attacker, hostile nanobot only

**Pursuit Halo**:
The one-cell-wide band surrounding Swarm Tiles, including cells that touch only at a corner. It permits an existing defense response to continue briefly outside swarm territory but does not make nearby hostile entities Threats by itself or constrain Defender travel routes.
_Avoid_: Swarm territory, detection range, unlimited pursuit

**Gather Zone**:
A swarm's intent to extract resources from available deposits, each contributing work once per eligible swarm regardless of painted area; overlapping swarms extract from the same finite resource pool without exclusive ownership or reserved shares. Gather intent persists through depletion, with workers leaving when no useful work remains and returning if resources become available again.
_Avoid_: Mining zone, resource zone

**Resource Deposit**:
A physical map object containing extractable resources for gather work, distinct from the resource kind it contains. It blocks Nanobot movement while the object exists, including when depleted; gathering happens from reachable space outside its footprint.
_Avoid_: Mineral node, mineral patch, resource pile

**Rock Formation**:
Permanent, impassable terrain that shapes base entrances, travel routes, and mineral pockets. Rock Formations cannot be destroyed or harvested and enclose the default map's playable area.
_Avoid_: Destructible rock, Resource Deposit

**Build Zone**:
A swarm's intent marking space where automatic construction may place support structures, providing placement options without creating construction demand or reserving space. Overlapping swarms may each build in physically free space, subject to structures and construction reservations of every swarm; paint never transfers structure ownership.
_Avoid_: Construction group, builder assignment, manual building placement

**Defend Zone**:
An intent zone that distributes unengaged Defenders at equal density and supports continuous density-driven, procedural roaming within and between its cells. It positions rather than bounds defense or creates population demand: Threats override staging, paint changes rebalance the cohort, and fallback staging uses Swarm Tiles or current cells.
_Avoid_: Fighter group, combat squad, attack zone

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
A Defender sustain resource restored by the nearest valid supplied Charger in owned Defend paint. New rotations are capacity-limited and release any Threat response; accepted rotations may finish travel and charging after casualties reduce the cap, unsupported Defenders continue duty while weakening, and recharged Defenders re-enter current allocation without reclaiming prior work.
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

**Swarm Elimination**:
A swarm's loss of all its Nanobots and completed structures, excluding Planned Structures. Any surviving Nanobot, Production Facility, Stockpile, or Charger prevents elimination regardless of production or recovery potential.
_Avoid_: Production Collapse, population wipeout, king unit death

**Match Outcome**:
The permanent result established by the first Swarm Elimination: the surviving side wins, or simultaneous elimination is a Draw. Standard presents the result as player Victory, Defeat, or Draw; AI Battle identifies the winning swarm or Draw, and Sandbox has no Match Outcome.
_Avoid_: Production status, recoverability warning

**Automatic Construction**:
The swarm creates needed structures from demand pressure rather than direct player placement. Production facilities, stockpiles, chargers, and similar support structures emerge inside or near matching intent paint when existing capacity is too busy for current intent. Painting a Build Zone alone does not create a structure; there must be active demand for the resulting support structure.
_Avoid_: Manual building placement, blueprint palette

**Minimum Category Activation**:
When an intent category has valid work and available eligible nanobots, the swarm should keep at least some work active in that category before letting normal scoring optimize the rest. This makes newly drawn valid intent visibly receive workers quickly without requiring direct unit control.
_Avoid_: Manual assignment, hard quota, perfect allocation

**Planned Structure**:
A stable, owner-scoped commitment to build one support structure at one location, persisting when its triggering demand recedes. It remains traversable until clearing for completion and resolves by completion, destruction, or cancellation when the final access check fails.
_Avoid_: Blueprint, ghost building, construction order

**Building Footprint**:
The visible world area reserved by a planned or completed support structure, identical for both forms and excluding overlap with other structures or Resource Deposits. Completed footprints block Nanobot movement; Planned Structures allow passage until clearing, and transient Nanobot occupancy is handled by clearing rather than placement exclusion.
_Avoid_: Generic sprite size, unit collision

**Structure Clearing**:
The phase after construction work and access validation in which a Planned Structure bars new entrants while existing occupants leave its Building Footprint. It becomes operational only after the footprint is empty.
_Avoid_: Forced eviction, instant completion

**Maintenance**:
Ongoing worker time required to prevent structure collapse. A structure remains fully functional while any health remains and is destroyed at zero; overexpansion or cut-off worker access creates collapse risk rather than partial shutdown.
_Avoid_: Permanent buildings, fire-and-forget construction

**Overlapping Intent**:
Independent intent zones covering the same space, including the same kind owned by different swarms. Each swarm acts on its own orders through autonomous allocation; overlap neither merges ownership nor creates a capture contest.
_Avoid_: Exclusive zones, Defend Contest, shared unowned paint

**Intent Allocation**:
The moment-to-moment act of steering the swarm by adjusting intent zone placement and size; players do not prioritize individual tasks or control population composition directly. Autonomous allocation weighs useful work, distance, type fit, crowding, and commitments, making Intent Allocation the primary player skill rather than individual nanobot management.
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
A player-painted movement intent for haulers that encourages resource transport along a path between stockpiles, facilities, chargers, or other resource needs. Owned Corridor cells apply a fixed route bias; their shape defines the preferred path, while cells without that swarm's Corridor paint give no benefit. Corridors do not create resource tasks by themselves and are special hauler guidance, not general direct movement commands.
_Avoid_: Road, waypoint chain, manual route

**Defender**:
A nanobot type that stages and roams in Defend Zones, responds to Threats throughout its swarm's Swarm Tiles, and may briefly pursue them through the Pursuit Halo.
_Avoid_: Fighter, soldier, combat unit

**Population Demand**:
The per-Nanobot-Type capacity justified by actionable workload: gathering, construction, and maintenance require Workers, physical transport requires Haulers, and Defender demand is the greater of half the swarm's unique Swarm Tile count rounded up and its active Threat count. Every swarm serves its greatest relative shortage, breaking equal ratios by the larger missing count and then stable type order; existing and committed capacity counts only for its own type, excess nanobots remain, and a pending Production Facility may increase Worker demand but never justifies another facility.
_Avoid_: Population cap, unit quota

**Production Pressure**:
Consecutive unmet Population Demand of any type while every operational Production Facility remains busy producing, or while no Production Facility is operational; waiting for exit space does not qualify as busy production. Resolved demand or idle operational capacity clears it, while brief demand spikes and a facility's internal cycle progress do not establish it.
_Avoid_: Build-zone size, instantaneous deficit

**Production Facility**:
A swarm-owned terminal support structure that consumes delivered resources and automatically fills its swarm's typed Population Demand. Production Pressure may create one unfinished expansion commitment per swarm; capacity is reassessed after that commitment completes or is lost.
_Avoid_: Barracks, factory queue, manual spawner
