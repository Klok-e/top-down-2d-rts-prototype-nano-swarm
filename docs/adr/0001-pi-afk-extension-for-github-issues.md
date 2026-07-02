# Use a Pi AFK extension for GitHub issues

Superseded in part by [ADR 0006](./0006-run-afk-roles-as-sdk-role-sessions.md) for AFK role execution.

We will replace the local `scripts/run_opencode_afk.sh` flow with a project-local Pi extension that runs one GitHub issue at a time through implementer, quality, and verifier roles. The extension owns issue selection, durable `.pi/afk/state.json` state, GitHub label/comment/close transitions, and final commit validation because those concerns need command UX, filesystem state, and deterministic GitHub/Git control rather than ad-hoc chat orchestration.

Rejected alternatives: dynamic workflows are a poor fit for durable command state and GitHub/Git side effects; `.scratch` issue files conflict with this repo's GitHub issue source of truth; `.pi/agents/afk-*` would expose one-off AFK roles as general task-solving agents, so AFK role prompts live under `.pi/afk/prompts/` instead.
