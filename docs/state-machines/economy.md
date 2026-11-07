# Economy and work state machines

These machines describe authoritative components and predicates. Named phases below are derived unless an enum is explicitly named.
Movement, blocked work, regional acquisition, and lease renewal are described in
[movement and allocation](movement-and-allocation.md). Session reset and visible
cancellation effects are in [runtime and presentation](runtime-and-presentation.md).

## Physical cargo and reservations

The owning code is [`Cargo`, `LogisticsReservation`, `MaterialWorkProgress`,
`nanobot_death_cleanup_system`](../../src/nanobot/cargo.rs).

- **Unreserved → reserved** begins when acquisition claims a source quantity and destination
  capacity. `amount` records the transfer budget, `source_remaining` is the
  uncollected portion, and `destination_remaining` is the capacity claimed at the
  current destination. That claim can cover only the next delivery segment when
  a retained physical load exceeds the destination's available capacity.
  Reservation creation changes neither physical minerals nor the resource ledger.
- **Reserved → partially loaded → carrying** tracks how each pickup changes source and cargo
  quantities. Deposit extraction adds minerals to the receiving swarm's ledger. Stockpile pickup only changes custody. Pickup completion releases unused source
  claims and bounds destination claims by the actual load.
- **Carrying → partially delivered → empty** tracks delivery, which changes cargo and endpoint
  quantities and reduces the destination claim. Exhausting a segment claim with
  cargo remaining returns the load to reroute/return/wait for another segment.
  Delivery does not change swarm totals.
- **Invalid endpoint → unclaimed load → redirected/returning load** releases
  claims independently of cargo. Cargo remains physically on the Nanobot.
- Material pulses use `MaterialWorkProgress.ticks_elapsed`. Active gathering and
  Hauler transfers move the configured per-pulse quantity after `logistics.material_work_ticks` fixed
  ticks. A pulse resets the counter. Changing phase removes it, and leaving the
  interaction region resets it. Blocked work does not advance it.
- Empty `Cargo` represents the start of loading. Finishing
  pickup without minerals removes Cargo and reservation. Regional lease revocation
  may also remove empty Cargo and partial work. Positive Cargo survives revocation.
Reservations coordinate custody changes while transfer systems perform them.
Retained coverage includes [physical Worker gathering](../../tests/behavior/physical_worker_gather.rs),
[gradual Hauler pickup](../../tests/behavior/gradual_hauler_pickup.rs),
[physical logistics playtest](../../tests/playtest/physical_logistics_flow.rs).

## Live economy reconciliation

After a tuning value is saved and published, reconciliation updates retained state for every swarm before the edit returns. [Publication lifecycle](runtime-and-presentation.md#live-tuning-publication-and-editor).

- Construction, production, Production Pressure, Maintenance shifts, material work, Charge pulses, and empty-Charge damage pulses preserve their completed fraction when their duration changes, with integer rounding. Incomplete work stays below completion; completed work remains complete and follows its normal completion/exit-space transition.
- Nanobot and structure maximum-health changes preserve living health fraction with at least one health; dead entities remain dead. Charge maximum changes preserve Charge fraction. Maintenance age preserves its buffer fraction, or completed overdue degradation steps plus its partial interval.
- Stockpile, production input, Charger, and carry limits change without deleting deposits, stored minerals, or physical cargo. Overfull endpoints refuse additional receipts until space becomes available. Existing funded production retains its target and paid cost.
- Reservation reconciliation admits loaded deliveries before future pickups, then uses stable entity order. Future pickup claims shrink to carry and destination capacity; empty jobs with no claim lose loading state and movement. Loaded jobs retain all cargo and can claim a partial delivery segment when the load exceeds newly available destination capacity. Delivery is bounded by that claim and free space; exhausting it leaves the remaining cargo physical and triggers another reroute/return/wait transition. No destination space leaves the load waiting. A limit change neither creates a transfer nor changes the swarm mineral ledger.
- Session replacement removes jobs and physical entities through their normal teardown, while the process retains tuning for the replacement scenario.

Owners are [economy hooks](../../src/nanobot.rs), [cargo reconciliation](../../src/nanobot/cargo.rs), [production](../../src/nanobot/production.rs), [Maintenance](../../src/nanobot/maintenance.rs), and [Charge](../../src/nanobot/charge.rs). Verification covers [economy tuning](../../tests/behavior/economy_tuning.rs), [combat tuning](../../tests/behavior/combat_tuning.rs), and [live logistics tuning](../../tests/behavior/live_logistics_tuning.rs), including `live_smaller_stockpiles_accept_existing_worker_and_hauler_loads_in_segments` for delivery across reduced-capacity buffers.

## Worker gather and delivery

The owning code is [`GatherAssignment`, `ExtractProgress`, `ReturningToStockpile`,
`worker_gather_*` systems](../../src/nanobot/gather.rs), and
[`regional_allocation_acquisition_system`](../../src/nanobot/allocation/runtime.rs).
`WorkerLoad` aliases Cargo.

| Derived phase | Encoding and transition |
| --- | --- |
| Available | Regional acquisition can assign eligible Worker to an actionable owned Gather opportunity. |
| Approach deposit | `GatherAssignment` plus movement toward the deposit's exterior interaction region. |
| Wait for delivery support | At deposit without extraction state. No usable reachable source-stockpile capacity yet. Pending navigation preserves this state. An existing source plan can release the assignment so the Worker can build. |
| Extract | Arrival reserves up to `logistics.worker_carry_capacity` minerals, bounded by unclaimed deposit quantity and destination capacity, then adds `ExtractProgress`, empty Cargo, and material progress. |
| Carry | Pickup budget fulfilled, source exhausted/missing, or approach confirmed unreachable causes the system to remove extraction, material progress, and gather assignment. Retain a positive load and reservation. |
| Deliver | `ReturningToStockpile` and movement to a compatible stockpile. After arrival, each eligible fixed tick moves up to `logistics.hauler_transfer_per_pulse` minerals, bounded by the segment claim and free space. |
| Available again | Last delivery removes Cargo, reservation, and return marker. A zero-load extraction exit also removes Cargo and reservation. |

Reroute and carry-assignment systems keep the load when the destination disappears,
fills, changes kind/ownership, stops operating, or becomes unreachable. They select
reachable compatible non-Sink stockpiles, counting competing reservations and
same-tick claims. A valid destination with pending navigation retains its claim. A confirmed invalid destination releases capacity while searching. No replacement
means waiting with cargo. Displacement reissues an
approach to the same interaction region. Extraction resets its partial pulse.

Gather paint survives depletion. Deposits remain physical blockers while depleted. Refilled deposits can become actionable again. Extraction only begins after a
built delivery endpoint exists. A Planned Structure cannot receive cargo.
Worker delivery has no `MaterialWorkProgress` gate. Its per-tick cadence differs
from material-pulse pickup and Hauler delivery. Automatic construction below also
uses Worker time rather than the material-pulse counter.

Retained coverage includes [gather behavior](../../tests/behavior/gather_zone.rs), [ownership](../../tests/behavior/gather_owner_filter.rs),
[overlap](../../tests/behavior/gather_overlap.rs), and physical Worker gathering.

## Hauler logistics

The owning code is [`HaulerAssignment`, `HaulerLoading`, `ReturningCargo`,
`hauler_*` systems](../../src/nanobot/haul.rs),
[terminal selection and acquisition](../../src/nanobot/allocation/runtime.rs),
and [leg compatibility](../../src/nanobot/logistics_leg.rs).

| Derived phase | Encoding and transition |
| --- | --- |
| Available → approach pickup | Acquisition writes assignment, exact reservation, and source approach movement. Capacity is up to `logistics.hauler_carry_capacity` minerals. |
| Loading | Source arrival adds `HaulerLoading`, empty Cargo, and material progress. Each completed pulse physically collects one mineral. |
| Carrying | Budget fulfilled or source exhausted/invalid/unreachable causes the system to remove loading and material progress. Retain positive Cargo and assignment, release remaining source quantity, and match destination claim to cargo. |
| Delivering | Valid destination approach finishes. Delivery creates material progress and transfers one mineral per completed pulse into stockpile, facility hopper, or Charger buffer. |
| Returning | `ReturningCargo` marks delivery to a compatible stockpile when ordinary downstream delivery cannot be found. |
| Available again | Empty pickup removes Cargo, assignment, and reservation. Last delivery additionally removes return marker and material progress. |

Normal acquisition admits source-stockpile → sink-stockpile and sink-stockpile →
terminal legs. Downstream service ranks urgency, waiting fairness, and route cost.
Corridor paint biases movement without creating work. Defensive manually seeded
Hauler assignments can still extract from deposits, but normal acquisition does
not select deposits or terminals as pickup sources.

`HaulerAssignment.pickup_role` records the original stockpile tier. Rerouting and
delivery use that stored role even after the source disappears. They never infer
a new tier from a missing source. Ordinary delivery must respect that original
leg. Physical return permits compatible stockpile custody instead.

Rerouting checks owner, resource kind, operational condition, outstanding claims,
free capacity, and reachability. A valid current route may wait for navigation.
Replacement selection prefers ordinary delivery over return and uses distance and
entity identity for stable ties. Pending ordinary routes postpone fallback return.
If no route can be accepted, destination claim and movement are released while
Cargo and original pickup tier remain. Delivery revalidates before each transfer. Competing deliveries update same-tick claimed capacity as minerals arrive.

Retained coverage includes [stockpiles and hauling](../../tests/behavior/stockpile_and_haul.rs),
[exterior hauling](../../tests/behavior/exterior_haul.rs), gradual Hauler pickup,
[terminal priority](../../tests/behavior/terminal_logistics_priority.rs), and
[Corridor behavior](../../tests/behavior/hauler_corridor.rs).

## Construction Worker and Planned Structure

Demand comes from [source-stockpile demand](../../src/nanobot/gather.rs),
[sink demand](../../src/nanobot/planned.rs), facility pressure below, and
[Charger demand](../../src/nanobot/charge.rs). Source demand requires active Gather
work without nearby usable source capacity or a pending source plan. Sink demand
serves nearby pending/completed facilities within owned Build paint. Low-Charge
Defenders without valid completed or pending service create Charger capacity
commitments in owned Defend paint. Raw paint alone creates no structure.

The owning code is [`PlannedStructure`, claims, work, clearing, and activation](../../src/nanobot/structure_lifecycle.rs).
[`PlannedKind`](../../src/nanobot/planned.rs) explicitly enumerates Source Stockpile,
Sink Stockpile, Production Facility, and Charger. It encodes structure kind.

Worker phases are **available → claimed/approaching → working → released**.
`try_claim` accepts only a plan with remaining work and no different active Worker.
`assign_worker` records `active_worker`, installs `PlannedStructureClaim`, and
approaches the exterior region. Arrival adds `PlannedStructureProgress`. Each
unblocked tick in-region spends one `work_remaining`, recording lease progress.
Displacement preserves the claim and reapproaches. Missing targets remove Worker
markers. Reconciliation clears stale `active_worker` references before projection,
so death or lease revocation makes remaining work available to another Worker.
Completion immediately releases claim, progress, lease, and the active Worker. Clearing consumes no continuing labor and no minerals.

The plan has the following outer lifecycle.

1. **Demand absent → committed**. Source, sink, facility, or Charger demand finds
   a permitted site and spawns an owner-scoped plan. Placement rejects overlapping
   deposits/structures/plans and unsafe friendly access. Nanobot occupancy is not
   a placement exclusion. Ordinary plans reserve their footprint but allow passage.
2. **Committed/unclaimed ↔ claimed/working**. The private work counter persists
   across Worker changes and receding demand. Completion is `work_remaining == 0`.
3. **Work complete → clearing**. Work installs `StructureClearing`. No terminal or
   stockpile functionality exists yet. `ClearingValidation` explicitly encodes
   `AwaitingInitialValidation` versus `Clearing { validated_layout }`.
4. **Awaiting validation → entry barred**. Access `Pending` waits. `Accepted`
   records the layout key and bars new entrants. `Rejected` cancels the plan.
5. **Entry barred → operational**. Occupants leave while the plan remains. Access is rechecked after relevant layout/intent changes and immediately before
   empty-footprint activation. Pending checks wait, rejected checks cancel, and
   accepted empty sites promote on the same entity with owner and transform intact.
6. **Cancelled → absent/excluded site**. Despawn releases footprint and commitment,
   clears associated evacuation state, and creates a visual-only cancellation effect.

`StructurePassage` explicitly derives `Traversable`, `EntryBarred`, or `Solid`
from plan/clearing presence. Entry barring has no congestion timeout. Evacuation
movement and its cleanup are detailed in [movement and allocation](movement-and-allocation.md).
[`CancelledSites`](../../src/nanobot/construction_access.rs) remembers owner, kind,
footprint, and blocker/plan signature. Its own plan removal cannot lift exclusion.
A changed layout permits reassessment. Combined-plan access protects previously
connected friendly useful endpoints and builder reachability. Enemy access receives no protection.
Promotion removes plan and clearing, installs an empty kind-specific operational
payload and completed visuals, and publishes spatial changes in lifecycle Commit.

Retained coverage includes [planned structures](../../tests/behavior/planned_structure.rs),
[access](../../tests/behavior/construction_access.rs),
[clearing](../../tests/behavior/structure_clearing.rs),
[cancellation playtest](../../tests/playtest/construction_cancellation.rs), and
[adjacent clearing playtest](../../tests/playtest/adjacent_structure_clearing.rs).

## Completed condition and Maintenance Worker

The owning code is [`Structure`](../../src/nanobot/build.rs), aliased as `SupportCondition`,
and [maintenance systems](../../src/nanobot/maintenance.rs).
Completed stockpiles, facilities, and Chargers acquire condition through observers.
**Healthy / needs Maintenance / overdue / collapsed** are overlapping predicates
and counters. Any positive health is fully operational. Damage or
`maintenance.needs_threshold` elapsed ticks makes a maintenance target.
Each degradation tick increments age. After the buffer, interval boundaries remove
health, and reaching zero despawns the structure. Combat destruction is another
exit. There is no damaged-throughput phase or recoverable collapsed state.

Worker **approach → work → release** uses `MaintenanceAssignment` and
`MaintenanceProgress { ticks_worked }`. Regional acquisition chooses eligible
same-swarm useful targets. Arrival and work require the exterior interaction region.
Each work tick restores bounded health, resets maintenance age, and advances shift
and lease progress. Displacement reapproaches without advancing work. Blocked work
waits. Shift completion or missing target removes assignment/progress. Maintenance
precedes degradation. The age may become one afterward but no overdue loss occurs.
It consumes Worker time without mineral consumption. Retained coverage includes
[maintenance](../../tests/behavior/maintenance.rs).

## Production cycle and expansion pressure

The owning code is [`ProductionFacility`, `ProductionPressure`, production systems](../../src/nanobot/production.rs).
**Idle** is `current_target == None`. **producing** has a target and progress below
`production.ticks_per_bot`. **waiting for exit** has completed progress and
`finished_at`. These are field predicates. `is_busy` excludes waiting for exit.
A funded operational facility with a valid owner selects current typed shortage,
consumes its full local hopper cost and ledger minerals once, then retains that
type despite later demand changes. Existing Nanobots and committed output count
only toward their own type. Same-pass selections include earlier commitments.
Progress advances while operational. Completed output waits for a physically
clear exterior cell, ordered by finish tick and entity. Successful spawn clears
target, progress, and finish timestamp. No exit means no repeated charge or spawn.
Destruction discards cycle/output. Invalid owner prevents output release.

Expansion pressure uses a per-swarm consecutive-tick counter independent of cycle progress.
Typed shortage plus every operational facility busy advances it, including zero
operational facilities. Resolved shortage, idle/exit-waiting operational capacity,
or an existing facility plan clears it. At threshold, valid owned Build placement
creates at most one unfinished facility commitment and resets pressure. Failure
to find space retains observation. Plan loss/completion begins a fresh window.
Retained coverage includes [production](../../tests/behavior/production_facility.rs),
[facility plans](../../tests/behavior/production_facility_planned.rs), and
[exits](../../tests/behavior/production_exits.rs).

## Defender Charge rotation and empty-health pulses

The owning code is [`Charge`, Charger markers, rotation and pulse systems](../../src/nanobot/charge.rs).
**Serving duty → travelling to Charger → charging → current allocation** uses
`ChargerAssignment`, then additional `ChargerProgress`/`ChargerPulseProgress`.
Drain continues in every phase. Low-Charge candidates are sorted by Charge, duty,
and identity. New rotations respect swarm capacity `max(living / 2, 1)` when any
Defenders live, zero capacity when none live, and three users per Charger. Accepted rotations survive later population-cap reductions.
Nearest reachable same-swarm supplied operational Chargers must lie in owned
Defend paint. Acceptance removes Threat response and lease. No available service
leaves current duty intact. Defender response itself belongs to movement/allocation.

Arrival starts a zeroed pulse counter. In-region unblocked work advances it. Displacement reapproaches and blocked work waits without resetting partial charge
pulse progress. Every `charge.pulse_interval_ticks`, consume up to `charge.material_per_pulse` available
minerals and grant `charge.per_pulse`, bounded by max. Full Charge, exhausted supply,
missing/invalid Charger, ownership loss, or erased Defend paint releases assignment,
progress, pulse, movement, and lease and wakes allocation. Previous duty is not restored.
Empty Charge outside rotation advances `EmptyChargeProgress`. Every `charge.empty_damage_interval_ticks` eligible
fixed ticks subtracts `charge.empty_health_damage` health. Positive Charge removes that counter. Rotation
excludes its advancement, preserving a preexisting partial empty-health pulse.
Weakening derives from the Charge multiplier.
Retained coverage includes [Chargers](../../tests/behavior/charger.rs),
[Charger plans](../../tests/behavior/charger_planned.rs), and terminal priority.

## Death, inactive legacy machinery, and decision boundaries

Nanobot **alive → zero health → settled/despawned** is owned by
[`nanobot_death_cleanup_system`](../../src/nanobot/cargo.rs). Changed zero Health
records death, removes exactly remaining Cargo from its swarm ledger, releases the
reservation, and despawns. Cargo already delivered is not lost again. Lease and
construction-owner reconciliation observe the disappearance.

The dormant build machinery has its own construction and repair lifecycle.
`structure_auto_creation_system` creates a `BuildSite` for painted Build cells
without a site or structure. `worker_build_assignment_system` chooses an idle
Worker's scored Build cell, preferring an unfinished site over a damaged structure,
then installs `BuildAssignment`, occupies a Build soft slot, and approaches the
exterior interaction region. Arrival adds `BuildProgress` and material progress.
These transitions belong to [`BuildPlugin`](../../src/nanobot/build.rs), which
[application setup](../../src/lib.rs) leaves unregistered.

While working on a site, each completed material pulse takes one mineral from
the nearest nonempty matching stockpile, removes that mineral from the default
ledger, and increments `consumed_materials` up to `required_materials`. While
repairing a Structure, the same pulse consumes a mineral and restores
`BUILD_HEALTH_PER_MATERIAL`, bounded by maximum health. The stockpile helper
searches all matching stockpiles without a distance or owner filter. **Material
wait** retains the assignment and progress at the target when no stockpile can
supply a pulse. Future pulses retry. Displacement resets partial material work
and reapproaches, while blocked work pauses advancement.

`BuildSite.is_complete` derives completion from consumed versus required material.
Completion removes BuildSite and inserts a fresh Structure on the same entity,
preserving its transform. Already-complete sites also promote when observed.
Full repair, site completion, or target disappearance releases the Build soft slot
and removes assignment, progress, and material progress. A target disappearing
before arrival removes the assignment. These dormant transitions use their own
site promotion and resource consumption rather than Planned Structure clearing.
Their helpers remain in the [build source](../../src/nanobot/build.rs).

[ADR-0005](../adr/0005-tiered-logistics-flow.md),
[ADR-0011](../adr/0011-owner-scoped-production-capacity-commitments.md),
[ADR-0012](../adr/0012-binary-structure-operation.md),
[ADR-0017](../adr/0017-construction-preserves-friendly-access.md), and
[ADR-0019](../adr/0019-demand-driven-automatic-production.md) define these contracts.
ADR-0019 supersedes ADR-0013's Production Priority. The glossary's destruction exit
for Planned Structures does not imply combat targetability. Plans lack completed
`Structure` condition and are not combat targets. Automatic plan construction
spends no minerals, so cancellation has no refund transition.
