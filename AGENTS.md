# Repository Guidelines

## Change Verification

- Review the diff for correctness, scope, and code quality.
- Run `cargo fmt`.
- Run `cargo clippy`.
- Run `cargo test`.

## Testing

For coding-agent implementation work, follow `docs/agents/testing.md`. Don't edit this doc autonomously: human approval always required.

## Agent skills

### Issue tracker

Issues and PRDs live as GitHub issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles mapped 1:1 (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout — `CONTEXT.md` at the root and `docs/adr/` (created lazily by the producer skill). See `docs/agents/domain.md`.
