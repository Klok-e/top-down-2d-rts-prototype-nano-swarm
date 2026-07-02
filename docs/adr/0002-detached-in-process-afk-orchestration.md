# Detached in-process AFK orchestration

Superseded in part by [ADR 0006](./0006-run-afk-roles-as-sdk-role-sessions.md) for AFK role execution and live transcript ownership.

AFK issue runs will start from `/afk run` or `/afk resume` and then continue as an in-process detached orchestrator so normal Pi commands remain usable while the run progresses. AFK pauses on Pi reload or shutdown instead of surviving independently.

## Considered Options

- Keep `/afk run` attached until completion: rejected because it blocks other commands during long runs.
- Own an external runner process: rejected for now because it is more robust but substantially more complex.
- Patch `@tintinweb/pi-subagents` for silent AFK-owned agents: rejected because local package changes would be overwritten on update.

## Consequences

The AFK extension must track an active in-memory run, reject concurrent runs, update durable state with `running` versus `paused`, and inject a guardrail so the main chat treats AFK progress as status unless the user explicitly asks it to intervene.
