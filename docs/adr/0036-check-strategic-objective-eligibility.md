---
status: accepted
---

# Check eligibility before ranking strategic objectives

The Strategic Controller considers objectives that meet their eligibility conditions in the current game state. It orders eligible objectives by priority and compares scores within a priority. This preserves explicit precedence for urgent objectives and lets the controller pass on plans the swarm cannot support.

A raid is eligible when its approach is reachable, the swarm has a minimum force of available charged Defenders, and that force is strong enough for the opposition along the approach and near the target. Health and Charge contribute to the strength comparison.

An enemy's last completed structure is eligible when the swarm can reach and attack it. The controller gives that target a bonus for ending the match and compares its score with other eligible offensive targets in the same priority tier. [Swarm Elimination](0033-structure-only-swarm-elimination.md) establishes which structures count.

Credible attack or sustained disruption at an Owned Resource Deposit makes it eligible for a strategic defense response. Routine intrusions use local Defender responses. When several mines qualify, the controller ranks immediate danger first and then the mine's importance to its economy.

The controller evaluates expansion and military objectives independently. An eligible expansion and an eligible raid can proceed together when their combined intent fits the planning and paint budgets.
