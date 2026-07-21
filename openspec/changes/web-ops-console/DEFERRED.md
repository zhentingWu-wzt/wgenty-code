# Deferred: web-ops-console

**Status**: deferred (2026-07-18)  
**Reason**: P0 personal ops console postponed; product priority is TUI polish. Web is repositioned as a **future team control plane**, not a single-user admin CRUD shell.

## Decision summary

- Do **not** implement personal-only Web ops console (sessions/memory/config) in the near term.
- Coding execution surface remains **CLI/TUI** (and later IDE).
- Future Web should target **team observability + collaboration**, with personal session/memory/context as a member-scoped view inside that product—not the whole product.

## Superseding product direction (sketch)

See conversation 2026-07-18. Layers:

1. **Local runtime** (today): TUI/CLI, project sessions, memory, daemon API as machine-local control plane.
2. **Identity + project binding**: user, team, repo/project_id on all exported events.
3. **Telemetry / turn store**: opt-in structured turns (not full raw prompts by default).
4. **Team hub (Web)**: roster activity, shared artifacts, reviews—not a second IDE chat.

## Artifacts retained

- `proposal.md` / `design.md` / `tasks.md` / delta specs remain as reference for API shapes if a thin local ops API is ever revived.
- Do not run `/comet-build` on this change until product re-approves scope.
