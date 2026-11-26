---
status: accepted
---

# Direct Source delivery and break-even Sinks

Amends [ADR-0005](0005-tiered-logistics-flow.md), under which terminals received minerals only from Sink Stockpiles, and [ADR-0033](0033-economy-driven-gathering.md), under which a terminal drew only from Sinks near its nearest stocked Sink and every delivered mineral passed through a Sink.

A Hauler moves one mineral per 24 ticks at each end of a leg, so loading or unloading a 10-mineral load takes 4 s, as long as 1,200 units of travel. Routing through a Sink adds an unload and a load and never shortens the path, so it always costs more Hauler time than delivering from the Source. In AI Battles Haulers spent about half their time on Source-to-Sink legs while Sources sat full and facilities starved, and Sinks landed 228-664 units from their facilities, often nearer the Source.

- **Direct legs.** Production Facilities and Chargers draw from same-owner Source or Sink Stockpiles, and haul ranking picks the cheaper pickup per mineral. The nearest-pickup rule and a facility's first delivery-time estimate treat Sources and Sinks alike. Gatherers still drop loads only at Sources.
- **Break-even Sinks.** A facility gets a Sink only when the nearest completed, operational own Source is farther from it, by walking path, than the Sink's spot by more than a break-even margin: the Hauler travel the Sink saves per second must repay its Maintenance. Both sides come from tuning: the full production rate, the Maintenance cadence, a fixed 3 s walk per Maintenance trip, and a Worker-second valued as a Hauler-second. The margin is about 70 units at current balance. The Sink takes the valid spot nearest its facility, and the check runs on the regional allocation cadence.
- **Unneeded Sinks decay.** A Sink that pays for no own facility whose 3x3 Build cells contain it requests no Maintenance and receives no Source-to-Sink legs. Terminals draw it down, and what remains is lost when it collapses. A Planned Sink persists and is built, as every Planned Structure is, then decays if it still does not pay.

Measured over Standard seeds 11-20 and Flanks seeds 42-46, 10 simulated minutes, as swarm means (Standard / Flanks):

| | Baseline | Direct legs, Sink per facility | No Sink within 500 of a Source | No Sinks |
|---|---|---|---|---|
| Bots built | 171 / 197 | 253 / 348 | 258 / 319 | 228 / 242 |
| Starved facility-seconds | 859 / 783 | 554 / 498 | 444 / 473 | 716 / 517 |
| Defender-seconds at empty Charge | 650 / 413 | 555 / 163 | 585 / 182 | 455 / 403 |
| Haulers at the end | 47 / 41 | 71 / 55 | 75 / 56 | 38 / 32 |

The implemented design, run alongside the unchanged baseline on the same seeds:

| | Baseline | This design |
|---|---|---|
| Bots built | 177 / 186 | 253 / 317 |
| Starved facility-seconds | 789 / 805 | 520 / 478 |
| Defender-seconds at empty Charge | 691 / 402 | 589 / 331 |
| Haulers at the end | 53 / 42 | 75 / 73 |

Run alongside a Sink per facility on Standard, it built 258 bots against 246, within noise. It ships regardless of that comparison. Flanks Defender time at empty Charge was 331 against 163 for a Sink per facility in an earlier, separate run; Standard showed no difference, and Flanks was not rerun.

## Considered Options

- **No Sinks.** Rejected: fewer bots built and more than twice the Defender time at empty Charge on Flanks. Hauler count halved as well, since pickup trips drive part of Hauler Population Demand, so part of the loss may come from Hauler supply rather than from Sinks.
- **A radius from hopper cover (about 1,200 units) or the reserve horizon (about 1,800 units).** Rejected: both remove every facility Sink on the current maps, which is the No Sinks result.
- **A fixed 500-unit radius.** It tied a Sink per facility but has no derivation.
- **Cancelling an unbuilt Sink plan that stops paying.** Rejected to keep Planned Structures persistent when their demand recedes.

## Consequences

- On the current maps the margin rarely skips a Sink, because a Sink at the nearest spot sits much closer to its facility than any Source.
- Each navigation geometry change drops the cached walking paths, and the next pass recomputes every facility's paths at once. Side by side with a Sink per facility, mean tick time was unchanged (3.42 vs 3.40 ms) but the worst tick rose from 30.5 to 36.8 ms, measured on a loaded machine.
- Two follow-ups remain open: terminals often take small loads from Sources that gatherers are still filling (15-60% of direct loads were 4 minerals or fewer), and Hauler Population Demand falls when Sink pickup trips do.
