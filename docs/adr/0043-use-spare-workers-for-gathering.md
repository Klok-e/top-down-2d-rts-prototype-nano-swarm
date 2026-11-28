---
status: accepted
---

# Use spare Workers for gathering

The Gather Quota sizes the workforce needed for income and reserves. A swarm that meets its Reserve Target can still have idle Workers and usable mining capacity. After quota gathering, construction, and Maintenance are staffed, offer remaining Workers Spare Gathering. Worker Population Demand follows required work. This amends Gather Quota enforcement in [ADR-0033](0033-economy-driven-gathering.md).

A spare trip needs a Work Position at an eligible Gather-painted Resource Deposit and a reachable, operational Source Stockpile in its drop-off area. Reserve space for a full load at that Source after accounting for required gatherers and existing claims. Required gathering determines when to plan new Source Stockpiles.

Release an empty Worker's spare trip and its claims when the Worker can cover reachable required work that would otherwise wait. A Worker carrying minerals finishes its delivery through the ordinary cargo rules. The delivered minerals contribute to reserves and Hauler Population Demand in the same way as minerals from quota gathering.
