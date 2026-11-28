# Repository Guidelines

## Constraints

Constraints not explicitly confirmed by the user may be relaxed to simplify code or improve readability, code quality, or performance. When you identify such an opportunity, explain the benefit and ask whether the user cares about preserving the particular constraint before relaxing it.

## Continuous Code Quality

On every implementation task, look for ways to improve the code you change and the repository code it directly depends on. Use your judgment to make worthwhile improvements that preserve behavior and fit the task's scope. Examples include renaming functions to reflect what they do and variables to reflect what they represent, removing unused symbols, simplifying and making code more elegant.

In the final response for each implementation task, name any significant refactor you deferred and explain its expected benefit.

## Papercuts and Maintenance

During any repository task, record papercuts you encounter that are likely to recur or straightforward to fix, along with actionable maintenance opportunities you notice, in [the maintenance log](docs/agents/maintenance.md). Give one bullet per distinct item with the affected area, concrete evidence, and a proposed improvement with its expected benefit. Update an existing entry when you have new information about the same item.

The maintenance log is the only file you may edit during a review, investigation, or other read-only task.

## Change Verification

- Review the diff for correctness and code quality.

## Temporary files

Use the repository-root `.tmp/` directory for agent scratch files and temporary verification artifacts. Create it when needed, use a unique subdirectory per run, and remove only your run's artifacts.

## Independent Cargo builds

Before the first Cargo build with a separate target directory on Btrfs, seed it from the main checkout's idle `target/`. Reflinking lets compatible build artifacts share disk space. Set `build_target` to the absolute path of the separate target directory and, while it is absent, run `cp -a --reflink=always -T -- target "$build_target"` from the main checkout. Set `CARGO_TARGET_DIR="$build_target"` for Cargo commands using that directory. If the source cache is unavailable, let Cargo create the directory. If the copy fails, remove only the partial destination created by that attempt before building.

## Testing

For coding-agent implementation work, follow `docs/agents/testing.md`.

For agent control and real-process verification, read `docs/agents/agent-control.md`.

## Agent skills

### Issue tracker

For issue and spec workflows, follow `docs/agents/issue-tracker.md`.

### Triage labels

When triaging, use the role mappings in `docs/agents/triage-labels.md`.

### Domain docs

When exploring domain behavior or recording a domain decision, follow `docs/agents/domain.md` for glossary and ADR guidance.
