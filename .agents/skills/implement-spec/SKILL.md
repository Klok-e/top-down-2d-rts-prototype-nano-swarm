---
name: implement-spec
description: "Implement the result of $to-spec and $to-tickets in code."
---

You have been provided a spec. This spec should have tickets associated with it, describing how to implement the spec.

Use the supplied issue tracker configuration or read `docs/agents/issue-tracker.md`. If neither is available, ask the user to run `$setup-matt-pocock-skills`.

The goal is the entire spec implemented on a single **integration branch**, with every ticket resolved the way the issue tracker closes work.

The tickets form a **task graph**. A ticket is ready once every blocker in the spec has merged and every external blocker is resolved in the tracker.

Start every subagent with a fresh context window.

Communication to and from subagents should be sparse. Communicate primarily through **context pointers**: to the spec, tickets, research notes, and previous commits. Don't duplicate information already available via pointers.

**Implementer subagents** should be run in the background where possible for maximum concurrency.

## Steps

1. Read the spec and tickets to understand the task graph.

2. (optional) Use an **exploration subagent** to conduct any exploration required by the tickets - relevant codebase files or external documentation. Give implementers the absolute paths to any exploration notes so they can read them from their worktrees. This lets **implementer subagents** focus on implementation rather than exploration.

3. Create the integration branch and record its starting commit. If the user authorized a PR, open a draft after the first merge in step 5 and note which issues the PR will close.

4. Create a named branch and a writable worktree from the current integration tip for each ready ticket. Fetch tracker requirements in the coordinating session and give each fresh **implementer subagent** the ticket contents, absolute worktree path, branch, and base commit. For real-process checks, assign each worker short, private `XDG_RUNTIME_DIR` and `XDG_CONFIG_HOME` directories and pass the process’s socket path to its client. Use a run-specific tuning copy when exercising tuning changes. If isolation is unavailable, schedule the process checks serially. Each implementer subagent:
   - uses the assigned worktree as the working directory for commands and the root for file edits. Before writing, verifies the path, branch, clean state, and base commit. Reports a mismatch to the coordinator; the coordinator repairs or replaces the assigned checkout;
   - builds the ticket one behavior at a time through the public interfaces named in the spec or the repository’s established test interfaces. Confirms that each test fails before implementation and passes afterward;
   - merges the integration branch tip into its own branch before reporting done

5. Merge completed branches serially in the integration checkout. If delegating a merge, give the merger the absolute checkout path and expected target branch.

6. After each successful merge, dispatch any newly ready tickets. If unfinished tickets remain with no work in flight and none ready, report the blocking tickets or dependency cycles.

7. Once all tickets are complete, run `code-review` on the integration branch using the recorded starting commit and the originating spec. Assign actionable findings to a fresh **implementer subagent** using the worktree procedure in step 4. Merge its fixes and verify the integrated result. Use the [code-quality handoff rule](../../../AGENTS.md#continuous-code-quality) when reporting deferred review findings.

8. For an authorized PR, push the verified integration tip and confirm the PR points to it. Mark it ready only when the user authorized a ready-for-review PR. When tracker updates are authorized, follow the configured resolution workflow for the spec and tickets that require closure outside the PR. Report the integration branch.

9. Remove run-owned worktrees after confirming their changes are integrated and they have no uncommitted work. Report any checkout that still contains work.
