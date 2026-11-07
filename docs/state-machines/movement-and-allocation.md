# Movement, allocation, and navigation state machines

These machines govern how Nanobots acquire useful work and reach it. Component
presence, retained local maps, counters, and cache entries encode state alongside
explicit enums. Independent machines compose. A loaded Hauler can retain a
Logistics Reservation, wait for standing space, and yield while navigation is pending.
Work and cargo lifecycles are described in [economy](economy.md).

## Exterior work approach and delivery queue

Owners include [`WorkApproaches`, `ApproachEntry`, `WorkApproach`, `ApproachPhase`,
`WaitingForWork`, `RejectedWorkGoal`, `work_standing_system`,
`move_velocity_system`](../../src/nanobot/work_navigation.rs).

- No interaction destination → no approach entry. A new `InteractionRegion` creates
  an entry with `since = now`, no selected point, no rejections, and `nearby = false`.
- The same region retains its age and candidate commitment through local searching
  and yielding. A changed region resets all of that state.
- `Travelling → Searching` when distance to the region's approach is within two
  navigation cells. An already-nearby approach stays nearby within three cells.
  This hysteresis controls occupancy checks and nearby point claims.
- A selected usable point is preferred. An already-valid current position is also
  a candidate. Otherwise choose an immediately clear candidate, then another
  statically valid candidate requiring a route. Nearby candidates must have
  separate body space and no earlier claim in this pass.
- Approaches are processed by original `since`, then stable entity identity.
  Nearby point claims implement oldest-approach priority without a separate queue.
- No free candidate → loaded bots and Defenders enter `Waiting`, retaining their
  work. Empty other bots abandon the assignment and lease, become Idle, and reject
  that work region for one second. Rejection expires on fixed simulation time.
- A confirmed unreachable selected point enters the entry's rejection list and
  clears selection. Navigation revision changes clear that list for another try.
- Arrival removes movement, progress checking, and observable approach/wait markers
  only at a clear, separate position. Recovery or overlapping bodies produce
  `WorkBlocked`, suspending work effects. Proximity alone never grants standing space.
- Entries are pruned when movement disappears. A session-generation change clears
  the local approach and route maps.

Coverage includes [work navigation](../../tests/behavior/work_navigation.rs),
[exterior work](../../tests/behavior/exterior_work.rs),
[congestion](../../tests/behavior/congestion.rs),
[physical logistics](../../tests/playtest/physical_logistics_flow.rs).

## Active route and retained shortcuts

Owners include [`ActiveRoute`, `move_velocity_system`](../../src/nanobot/work_navigation.rs)
and [`RouteFollower`, `rejoin_index`](../../src/nanobot/route_following.rs).

- No route → direct single-waypoint route when permitted, otherwise owned pending
  request. Haulers use direct final approach near work but preserve Corridor route
  preference on longer trips.
- A changed task, tolerance, goal, exhausted route, or obstructed next segment can
  request replacement. Small moving point-target changes may retain the old route.
- A topology revision rechecks segments and trims the route before its first
  obstruction. A replacement can retain the safe prefix for the same task while
  pending. Pending results do not authorize travel through blocked geometry.
- `Pending → Found` installs waypoints and rejoins the nearest reachable segment
  from the actual position, avoiding a return to the request's old start.
  `Pending → Unreachable` clears the route and rejects its approach candidate.
- The follower retains a chosen shortcut index until actual movement reaches it.
  Requesting a step does not consume skipped nodes. Invalid or newly blocked
  shortcuts are discarded. It examines at most eight upcoming waypoints and
  preserves Hauler turns instead of cutting across their Corridor path.
- Arrival, supersession, or loss of the movement component cancels the owned
  request. Dead/nonmoving entities leave the local route map. Living bodies
  losing their route also lose remaining-travel, waiting, and approach markers.

Coverage includes [route follower tests](../../src/nanobot/route_following.rs),
[worker route recovery](../../tests/behavior/worker_route_recovery.rs),
[shared navigation](../../tests/playtest/shared_navigation.rs).

## Congestion recovery, yielding, and local momentum

Owners include [`TravelProgress`, `LocalMotion`, `Yielding`, `TrafficYield`,
`CongestionRecovery`, `velocity_system`](../../src/nanobot/move_system.rs).

- Ordinary travel retains its best distance and last meaningful progress time.
  Improving by more than two units updates the checkpoint. Interaction-region
  distance prevents perimeter orbiting from appearing to be useful progress.
- After one second without progress, a traveller with remaining route distance
  obstructed by a friendly body enters recovery. Returning yield trails supply
  the obstruction direction when the original route direction is misleading.
- Recovery permits friendly-body transit overlap, never overlap with world
  geometry. It cancels yielding and suppresses normal separation for active travel.
- Recovery remembers whether bodies crossed. After crossing or renewed progress,
  becoming nonoverlapping restores normal travel and resets its checkpoint.
  No movement or no remaining route clears recovery unless recovery overlap persists.
- Head-on conflicts deterministically choose one bot to yield. Nearby followers
  can inherit its retreat. `Yielding` retains the priority body, retreat direction,
  chosen side, actual-motion trail, and a `returning` flag.
- Yielding sidesteps or retreats while traffic passes, then returns along its
  recorded trail. A clear route rejoin, safe normal motion, completed trail,
  changed goal, pinned work, missing participant, or recovery removes yielding.
- Idle/queued bodies also sidestep approaching friendly traffic. Active work holds
  its standing position. Yielding retains the original work approach's queue age.
- `LocalMotion.velocity` retains momentum for acceleration, braking, and turning.
  Collision constraints can stop immediately. Applied motion, rather than desired
  motion, updates position and regional-lease travel progress. Velocity input is
  consumed each pass. Generation changes clear motion, progress, and yielding maps.

Coverage includes [local avoidance](../../tests/behavior/local_avoidance.rs),
[congestion](../../tests/behavior/congestion.rs),
[movement](../../tests/behavior/movement.rs).

## Idle departure and spread

Owners include [`LocalMotion.departure`, `velocity_system`](../../src/nanobot/move_system.rs)
and [`SpreadTarget`, `idle_spread_system`](../../src/nanobot/spread.rs).

- An unqueued, unpinned bot without movement near another friendly work approach
  can choose a nearby clear departure point farther from that work perimeter.
  Retain it until arrival or obstruction. Renewed work/movement clears it.
- Generic spread applies only to Idle bots without movement or active task
  markers. Workers fit Gather/Build, Haulers fit Corridor, and Defenders use staging.
- `SpreadTarget` retains a cell, best distance, and progress time. Keep it while
  owned matching intent remains, arrival has not occurred, and progress has not
  stalled for one second. A two-unit improvement renews the checkpoint.
- Otherwise choose a less dense adjacent fit cell, deterministic exploratory
  neighbour on a flat field, or nearest fit cell when stranded. No candidate
  removes the target. Spread adds a velocity nudge without creating a work order.
- Density and fit indexes are per-pass snapshots. Target history and spread tick
  reset on session generation, and targets are pruned when bots cease to qualify.

Coverage includes [idle spread](../../tests/behavior/idle_spread.rs).

## Structure Clearing evacuation override

Owners include [`ClearingEvacuation`, `clear_finished_structures_system`](../../src/nanobot/structure_lifecycle.rs)
and [`move_velocity_system`](../../src/nanobot/work_navigation.rs).

- Validated clearing with an occupant → select a clear, unoccupied exterior exit
  reachable from that body's position, then insert evacuation and movement.
  An existing exit is retained only while still clear, free, and reachable.
- Evacuation temporarily overrides routing with Clearing priority. An independently
  changed work destination is preserved when the evacuation goal is reached.
- Leaving the footprint or losing the clearing structure removes evacuation.
  Movement is removed only when it is still the evacuation's own point order.
- Failed final construction access cancels the plan and removes its evacuation
  orders. No available exit leaves clearing waiting. Activation requires emptiness.

Coverage includes [structure clearing](../../tests/behavior/structure_clearing.rs),
[adjacent clearing](../../tests/playtest/adjacent_structure_clearing.rs).

## Regional lease, cadence, wake, and fairness debt

Owners include [`RegionalLease`, `LeaseTravel`, `evaluate_lease`, lease maintenance/release](../../src/nanobot/allocation/lease.rs),
[`AllocationClock`](../../src/nanobot/allocation.rs), and
[`AllocationTickDue`, `RegionalAllocationWake`, `RegionalServiceAges`,
`TerminalDemandAges`, acquisition](../../src/nanobot/allocation/runtime.rs).

- Successful acquisition creates a category/target/owner lease with a no-progress
  deadline. Its private monotonic progress counter belongs to that lease.
  Actual work records progress. Movement arms a waypoint checkpoint and only
  applied motion improving its best distance records travel progress.
- Measurable progress renews the deadline. Unsupported work revokes immediately.
  Otherwise reaching the deadline revokes for no progress. Pending connectivity
  is not proof of lost access, and a reservation can support a projected claim.
- Positive physical cargo bypasses this revocation path. Delivery retains custody.
  Empty Gather/Haul revocation removes Cargo and MaterialWorkProgress alongside
  assignments, reservations, movement, and category progress, then returns Idle.
  Finishing a category removes its lease when its active assignment disappears.
- Projection/invalidation run each fixed step. General acquisition runs initially,
  across 100 ms allocation-clock boundaries, or for an explicit wake. Defender
  response/staging reconciliation runs each fixed step independently of that gate.
- Wake is a one-shot boolean consumed every advance, including a step already due
  by cadence. It cannot survive short-circuit evaluation and spuriously wake later.
- Region/type/swarm service debt tracks actionable work for swarms with eligible
  acquisition candidates. Its `u32` ages saturate, reset on assignment, and are
  pruned when their keys leave that candidate set. At `autonomy.regional_fairness_promotion_ticks` acquisition passes it enters the
  promoted bounded-search window, then age, distance, and stable coordinates order it.
- Terminal debt similarly ages actionable terminal legs and resets on assignment.
  Urgency promotion subtracts `age / autonomy.terminal_fairness_promotion_ticks` in `u32` with saturation before conversion
  to `u8`. Older service debt cannot wrap back to a worse urgency tier.
- Empty projection clears both age maps. These ages measure acquisition passes,
  including wake passes, rather than elapsed wall-clock seconds.
- Allocation session reset replaces projection, territory, staging layouts, clock,
  due/wake flags, and both service-age maps with defaults. Session teardown removes
  entity-local assignments, routes, and leases. [Allocation reset](../../src/nanobot/allocation.rs),
  [session teardown](../../src/session_lifecycle.rs).

Coverage includes [regional allocation](../../tests/behavior/regional_allocation.rs),
[terminal logistics priority](../../tests/behavior/terminal_logistics_priority.rs).

## Live autonomy reconciliation

Category weights, fairness promotion thresholds, lease duration, controller review period, and strategic scoring parameters come from live tuning. Category weights change candidate scores without changing acquisition or navigation work budgets. Fairness ages remain acquisition-pass counts and use the current promotion thresholds.

Accepted relevant edits dirty existing projection regions and active intent cells and request an allocation wake. A lease-duration change scales its remaining no-progress deadline against the allocation clock; it retains progress history and cargo protection. The next normal projection/acquisition pass rebuilds opportunities under the new values. [Autonomy reconciliation](../../src/nanobot/allocation/runtime.rs), [lease deadline](../../src/nanobot/allocation/lease.rs).

Corridor-cost changes update future navigation searches; existing routes retain their normal validation and interruption rules. Session reset clears leases, service ages, and navigation requests while retaining process tuning. [Logistics reconciliation](../../src/nanobot/cargo.rs), [navigation costs](../../src/navigation/budget.rs), [live logistics coverage](../../tests/behavior/live_logistics_tuning.rs).

## Navigation requests, lazy hierarchy, and query caches

Owners include [`Request`, `Scheduler`, `WorkUnit`, `Navigation::advance`,
`query_connectivity`, `query_access`](../../src/navigation/budget.rs),
[`Navigation::refresh`](../../src/navigation/routing.rs),
[navigation runtime](../../src/navigation_runtime.rs).

- Request insertion creates Pending with no future. Scheduled service captures a
  geometry/paint snapshot and creates a search future. Bounded polling retains
  that future across passes until Found or Unreachable. Results remain until
  explicit cancellation or query-cache expiry. Polling an unknown id returns Pending.
- Movement and background connectivity/construction occupy separate lanes with
  three-quarter/one-quarter shares, fractional rotation, and borrowing of unused
  capacity. Priority boosts and time since service order each lane. Unfinished
  requests rotate after 128-poll quanta to share the allowance.
- Geometry revision makes old answers Pending, drops old futures, and restarts
  work against current geometry. Clearing changes also invalidate geometry state.
  Paint revision refreshes the shared cost snapshot for newly created futures.
  It does not revoke existing route commitments merely for a cheaper Corridor.
- A missing hierarchy chunk becomes a shared partial build, then a cached chunk.
  `WorkUnit` yields Pending once before Ready, making hierarchy cells, graph edges,
  and search iterations participate in the same scheduler allowance. Shared builds
  survive individual request cancellation. Build snapshots avoid cache ownership cycles.
- Geometry refresh preserves unaffected chunk results, removes affected chunks
  (all on bounds change), clears partial builds and connections, and increments
  revision. Navigation runtime session reset replaces the entire Navigation
  resource, discarding request, result, hierarchy, and paint caches.
- Connectivity queries reuse fine-component/goal keys, or guarded cold-cell keys
  before components exist. A cold key can migrate to its component key. Reuse
  requires current revision and compatible origin access. Access checks cache by
  check/revision and validate compatible builder origins before reuse.
  Repeated queries refresh their last-touch time. Stale revision or 120 untouched
  scheduler ticks expire an entry and its request.

Coverage includes [scheduler tests](../../src/navigation/budget.rs),
[navigation budget](../../tests/behavior/navigation_budget.rs),
[task reachability](../../tests/behavior/task_reachability.rs).

## Derived-state publication and other cache protocols

- `ActionableProjection` retains clean
  region opportunities plus a dirty set. Intent dirty cells, changed/dead targets,
  stockpile/terminal changes, and Defender maintenance-neighbour changes invalidate
  dependent regions. Projection drains dirty regions, rebuilds sorted opportunities,
  and replaces each region atomically within its system pass. Previous maintenance
  Defender locations are comparison history, cleared on generation change. [ActionableProjection](../../src/nanobot/allocation/projection.rs).
- `WorkOrigins` memoizes crew origins by swarm,
  type, and whether production exits count. A navigation-work tick or generation
  change discards memoized origins. Repeated queries within that tick reuse them.
  `WorkReachability` is a query outcome, not a persistent bot lifecycle. [WorkOrigins](../../src/nanobot/work_access.rs).
- `SwarmReachabilityMap` lazily memoizes a boolean for
  each swarm/cell against permanent terrain and scenario home. Answers persist for
  that scenario. Deposits, structures, and clearing do not invalidate this boundary. [SwarmReachabilityMap](../../src/reachability.rs).
- `TerritorySnapshot` is a derived view
  of Swarm Tiles, Threats, and Pursuit Halo, not an independent duty machine.
  Candidate records, route goals/priorities, spatial buckets, and opportunity enums
  likewise classify data rather than independently transition over time. [TerritorySnapshot](../../src/nanobot/allocation/territory.rs).

Coverage includes [actionable projection](../../tests/behavior/actionable_projection.rs),
[territory projection](../../tests/behavior/territory_projection.rs),
[per-swarm intent](../../tests/behavior/per_swarm_intent_ownership.rs).

## Defender duty, staging waypoints, and attack cadence

Owners include [`DefenderResponse`, response reconciliation](../../src/nanobot/allocation/defender_response.rs),
[`DefenderStaging`, `DefenderStagingLayouts`, staging reconciliation](../../src/nanobot/allocation/defender_staging.rs),
and [`DefenderAttackCooldown`, combat](../../src/nanobot/combat.rs).

- Available Idle, living Defenders without Charge service retain a valid pursuit
  or acquire an unclaimed Threat. One pursuit claims each target. Only a higher
  danger tier preempts. Existing pursuit may continue through the Pursuit Halo.
  Target death, departure, friendly ownership, or loss of availability releases it.
- Pursuit updates movement each fixed step and overrides staging. Charge admission
  releases duty. Return enters current allocation rather than restoring old claims.
- Unengaged cohorts balance owned Defend cells, otherwise Swarm Tiles, otherwise
  their current cells. Cached cohort/cell input preserves a valid layout. Changes
  recompute assignments with continuity and travel-cost preferences.
- Each staging assignment retains a cell, waypoint, and wrapping waypoint index.
  Arrival advances the index. Obstructed procedural points are skipped. Travel to
  the cell uses normal speed. Roaming within it uses the gentle spread speed.
- Steady layouts retain their assigned cells and cycle subcell waypoints. This
  conflicts with continuous density-driven cross-cell roaming promised by the ADR
  and glossary. Cohort/paint changes rebalance, but steady-state roaming does not
  independently change cells. [ADR 0015](../adr/0015-territory-wide-defender-response.md).
- Responding Defenders attack the nearest hostile in range, independently of their
  pursuit target. A delivered attack sets cooldown to interval minus one. Eligible
  responding combat passes otherwise decrement an existing cooldown toward zero.
  Nonresponding Defenders are outside that decrement loop.
- Gameplay death owns assignment/cargo/support cleanup. Resolved combat facts
  feed presentation. [Economy machines](economy.md),
  [presentation machines](runtime-and-presentation.md).

Coverage includes [Defender response](../../tests/behavior/defender_response.rs),
[Defender staging](../../tests/behavior/defender_staging.rs),
[combat](../../tests/behavior/combat.rs).
