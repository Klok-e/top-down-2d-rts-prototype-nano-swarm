# Use token-gated AFK result tools for subagent completion

AFK subagents will report implementer, quality, and verifier outcomes by calling AFK-owned terminating result tools instead of ending with manually formatted JSON text. The AFK extension will issue a per-spawn token, gate `afk_role_result` and `afk_verify_result` calls against an active token registry, and consume an atomic `.pi/afk/results/<token>.json` handoff because this preserves structured completion semantics without modifying `@tintinweb/pi-subagents` or relying on prompt-only JSON compliance.

## Considered Options

- Keep final JSON-line parsing: rejected because a successful subagent can pause AFK by ending with prose instead of parseable JSON.
- Spawn a repair/reformat agent on bad JSON: rejected because it treats structured completion as an after-the-fact recovery path rather than a first-class result channel.
- Modify `@tintinweb/pi-subagents` to add structured output: rejected for now because AFK can use Pi extension tools directly while avoiding package-local changes that may be overwritten on update.
- Use provider-native structured output: rejected for now because Pi/subagent provider support is not uniform, while a terminating tool is provider-agnostic.

## Consequences

AFK result tools will be visible as project extension tools but will only accept active tokens. Result files are transport artifacts: first write wins, active tokens are removed when the subagent completes, and valid result files are deleted after AFK consumes them.
