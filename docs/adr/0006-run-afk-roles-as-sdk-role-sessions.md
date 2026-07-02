# Run AFK roles as SDK role sessions

AFK will run implementer, quality, and verifier phases as AFK-owned Pi SDK role sessions instead of spawning them through `@tintinweb/pi-subagents` RPC. This removes AFK's hard dependency on `pi-subagents` for role execution while still allowing role sessions to use any normally loaded tools, including `Agent` when `pi-subagents` is installed; AFK injects only its token-gated result tools and excludes the full AFK extension from child role sessions to avoid recursive `/afk` orchestration.

## Considered Options

- Keep spawning AFK roles through `pi-subagents` RPC: rejected because `pi-subagents` intentionally removes nested `Agent` tools from subagent sessions, preventing AFK roles from delegating.
- Patch `pi-subagents` to allow nested agents: rejected because it couples AFK to package internals and risks runaway nested agent trees.
- Keep both RPC and SDK role runners: rejected because rollback can use git history, while dual runners preserve the old limitation and add state/test complexity.

## Consequences

AFK no longer uses `/agents` as its primary role-runner UI. Role sessions write AFK-owned transcripts under `.pi/afk/transcripts/`, `/afk stop` aborts the active role session only, and per-role model configuration accepts exact `provider/modelId` values only.
