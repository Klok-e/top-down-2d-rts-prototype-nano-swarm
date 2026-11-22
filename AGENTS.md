# Repository Guidelines

## Constraints

Constraints not explicitly confirmed by the user may be relaxed to simplify code or improve readability, code quality, or performance. When you identify such an opportunity, explain the benefit and ask whether the user cares about preserving the particular constraint before relaxing it.

## Change Verification

- Review the diff for correctness and code quality.

## Temporary files

Use the repository-root `.tmp/` directory for agent scratch files and temporary verification artifacts. Create it when needed, use a unique subdirectory per run, and remove only your run's artifacts.

## Git worktrees

When creating a worktree for Cargo builds on Btrfs, seed its private build cache before the first build. From the main checkout, with its `target/` idle and `$worktree` set to the new worktree's absolute path, run `cp -a --reflink=always -T -- target "$worktree/target"` while the destination is absent. Set `CARGO_TARGET_DIR="$worktree/target"` for Cargo commands in that worktree. If the source cache is unavailable, let Cargo create the directory. If the copy fails, remove any partial `$worktree/target` created by that attempt before building.

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
