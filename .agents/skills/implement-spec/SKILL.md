---
name: implement-spec
description: "Implement the result of /to-spec and /to-tickets in code."
disable-model-invocation: true
---

You have been provided a spec. This spec should have tickets associated with it, describing how to implement the spec.

Use the supplied issue tracker configuration or read `docs/agents/issue-tracker.md`. If neither is available, ask the user to run `/setup-matt-pocock-skills`.

The goal is the entire spec implemented on a single **integration branch**, with every ticket resolved the way the issue tracker closes work.

The tickets form a **task graph**. A ticket is ready once every blocker in the spec has merged and every external blocker is resolved in the tracker.

Start every subagent with a fresh context window.

Communication to and from subagents should be sparse. Communicate primarily through **context pointers**: to the spec, tickets, research notes, and previous commits. Don't duplicate information already available via pointers.

**Implementer subagents** should be run in the background where possible for maximum concurrency.

## Steps

1. Read the spec and tickets to understand the task graph.

2. (optional) Use an **exploration subagent** to conduct any exploration required by the tickets - relevant codebase files or external documentation. Give implementers the absolute paths to any exploration notes so they can read them from their worktrees. This lets **implementer subagents** focus on implementation rather than exploration.

3. Create the integration branch and record its starting commit. If the issue tracker closes work through PRs, or the user asks for one, open a draft PR after the first merge in step 5 (a branch with no commits ahead of main can't open one), marked as closing the spec and tickets.

4. Use a fresh **implementer subagent** for each ticket, each in its own worktree on its own branch. Each implementer subagent:
   - confirms its worktree is based on the integration branch before starting, and resets onto it if not;
   - builds the ticket one behavior at a time through the public interfaces named in the spec or the repository’s established test interfaces. Confirms that each test fails before implementation and passes afterward;
   - merges the integration branch tip into its own branch before reporting done

5. Once an **implementer subagent** completes, merge its work to the integration branch with a **merger subagent**. Serialize merges.

6. After each successful merge, dispatch any newly ready tickets. If unfinished tickets remain with no work in flight and none ready, report the blocking tickets or dependency cycles.

7. Once all tickets are complete, run `code-review` on the integration branch using the recorded starting commit and the originating spec. Assign actionable findings to a fresh **implementer subagent**. Merge its fixes and verify the integrated result.

8. If a draft PR exists, push the verified integration tip and confirm the PR points to it before marking it ready for review. Use the configured tracker’s resolution workflow for the spec and tickets that require closure outside the PR. Report the integration branch.

9. Clean up all **implementer subagent** worktrees.
