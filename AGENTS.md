# Repository Guidelines

## Change Verification

- Review the diff for correctness and code quality.

## Temporary files

Use the repository-root `.tmp/` directory for agent scratch files and temporary verification artifacts. Create it when needed, use a unique subdirectory per run, and remove only your run's artifacts.

## Testing

For coding-agent implementation work, follow `docs/agents/testing.md`. Don't edit this doc autonomously: human approval always required.

For agent control and real-process verification, read `docs/agents/agent-control.md`.

## State machines

When adding, changing, or removing behavior retained across ticks, frames, requests, or sessions, read the relevant entries in [the state-machine inventory](docs/state-machines.md). Update its index and affected entries in the same change, covering state encoding, transition guards, timing, interruption and cleanup, reset boundaries, and verification links. This applies to implicit component/flag/counter state and to conversions between implicit and explicit state machines.

## Agent skills

### Issue tracker

For issue and spec workflows, follow `docs/agents/issue-tracker.md`.

### Triage labels

When triaging, use the role mappings in `docs/agents/triage-labels.md`.

### Domain docs

When exploring domain behavior or recording a domain decision, follow `docs/agents/domain.md` for glossary and ADR guidance.
