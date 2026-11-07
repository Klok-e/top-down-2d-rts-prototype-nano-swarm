# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- Read `GLOSSARY.md` at the repo root. If the root has a `GLOSSARY-MAP.md`, use it to find and read the relevant glossaries.
- Read the relevant ADRs in `docs/adr/`. In multi-context repos, also check `src/<context>/docs/adr/` for context-scoped decisions.

If any of these files don't exist, proceed silently.

## Use the glossary's vocabulary

When output names a domain concept, use the glossary's preferred term.

If the needed concept isn't present, reconsider whether it belongs to the project or note the gap for `/domain-modeling`.

## Flag ADR conflicts

Surface contradictions with existing ADRs explicitly instead of silently overriding them.
