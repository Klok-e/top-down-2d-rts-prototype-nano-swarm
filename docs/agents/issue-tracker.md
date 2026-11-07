# Issue tracker: GitHub

Use the `gh` CLI for GitHub operations. Load its token with `source ./.env && export GH_TOKEN`.

Set and export `GH_REPO` to the `OWNER/REPO` from `git remote get-url origin` before running tracker commands. Verify the target with `gh repo view --json nameWithOwner`.

## Conventions

- **Publish a spec or ticket.** Run `gh issue create --title "..." --body-file <path>`.
- **Read an issue.** Run `gh issue view <number> --json number,title,body,author,createdAt,updatedAt,labels,comments`.
- **List issues.** Run `gh issue list --state open --json number,title,body,author,createdAt,updatedAt,labels,comments` with the appropriate state and label filters. When the count reaches the limit, raise `--limit` above its default of 30 until the result is complete.
- **Link a child issue.** Use `gh issue create --parent <parent> ...` or `gh issue edit <parent> --add-sub-issue <child>`. If sub-issues are unavailable, put `Part of #<parent>` at the top of the child issue and list it in the parent's task list.
- **List child issues.** Run `gh api --paginate 'repos/{owner}/{repo}/issues/<parent>/sub_issues' --jq '.[].number'` or read the parent's task list.
- **Link a blocker.** Use `gh issue create --blocked-by <blocker> ...` or `gh issue edit <child> --add-blocked-by <blocker>`. If native dependencies are unavailable, put a `Blocked by: #<n>, #<n>` line at the top of the child issue.
- **Count open blockers.** Run `gh issue view <number> --json blockedBy --jq '[.blockedBy.nodes[] | select(.state == "OPEN")] | length'`. For blockers listed in the body, check the state of each linked issue. Zero open blockers means the ticket is unblocked.
- **Comment on an issue.** Run `gh issue comment <number> --body "..."`.
- **Apply or remove labels.** Use `gh issue edit <number> --add-label "..."` or `--remove-label "..."`.
- **Close an issue.** Post the resolution as a comment, then run `gh issue close <number>`. The separate comment also works if the issue has already closed.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests. `/triage` reads this flag.)_

When set to `yes`, use these operations:

- **Read a PR.** Use `gh pr view <number> --json number,title,body,author,createdAt,updatedAt,labels,comments` and `gh pr diff <number>`.
- **List external PRs for triage.** Use `gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, created_at, updated_at, labels: [.labels[].name]}'`.
- **Comment, label, or close.** Use `gh pr comment`, `gh pr edit --add-label` or `--remove-label`, and `gh pr close`.

GitHub shares one number space across issues and PRs. Resolve a bare reference such as `#42` with `gh pr view 42`, then fall back to `gh issue view 42`.

## Wayfinding operations

- **Map.** Add `--label wayfinder:map` to the publish command under Conventions.
- **Child tickets.** Use the child issue and blocker operations under Conventions.
- **Frontier query.** Read each child's state and assignment with `gh issue view <number> --json state,assignees` and count its open blockers.
- **Claim.** Run `gh issue edit <number> --add-assignee @me`.
