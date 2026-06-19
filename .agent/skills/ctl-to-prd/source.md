---
name: ctl-to-prd
description: "Synthesize the current resolved context into a PRD — do NOT re-interview the user unless information is genuinely missing. Separates ObservedBasis (what the agent read) from ConfirmedBasis (what an authority confirmed) from OpenUncertainty (unresolved unknowns, never hidden), and carries a draft/confirmed/superseded status. Triggers when: enough context is resolved (often after ctl-grill-with-spec) and you are about to spin up multiple durable tasks. Do NOT trigger for: a single obvious task (go straight to ctl-to-tasks), or when key intent is still unknown (grill first)."
---


## Synthesize the PRD (phase body)

Scaffold the shape with the ctl CLI, then fill it from resolved context:

```
ctl prd init --title "<title>" > .ctl/spec/prd/<prd-id>.md
```

`ctl prd init` prints a structured PRD template (Objective / Context / Tasks).
Filling it is the "grill" step; the `## Tasks` section is the hand-off to
`ctl-to-tasks`. V1 is an **agent-readable artifact workflow only** — there is no
PRD subsystem, no PRD events, and the PRD never gates a task.

### The three bases (never collapse them)

Every claim in the PRD is tagged by where it came from:

- **ObservedBasis** — what the agent actually read or ran (cite the file/command).
- **ConfirmedBasis** — what the user or an existing project authority explicitly
  confirmed.
- **OpenUncertainty** — unresolved unknowns. These must be surfaced, never hidden;
  they travel into the task proposals as blocking uncertainties.

A belief with no observation and no confirmation is OpenUncertainty, not a
requirement.

### Status lifecycle

The PRD header carries exactly one status:

- `draft` — synthesized but not yet confirmed by the user/authority.
- `confirmed` — the user accepted it as the basis for task generation.
- `superseded` — replaced by a later PRD (link forward).

Stay in `draft` until explicitly confirmed. Do not generate durable tasks from a
PRD that is still `draft` unless the user asks for a dry run.

### Quality bar

- Every requirement is tagged ObservedBasis / ConfirmedBasis / OpenUncertainty.
- No OpenUncertainty was silently promoted into a requirement.
- The `## Tasks` section lists vertical, independently shippable slices.
- Status is set honestly; a draft is labelled a draft.

### Anti-patterns

- ❌ Re-interviewing the user for context already resolved in the grill.
- ❌ Presenting an assumption as a confirmed requirement.
- ❌ Hiding an unknown to make the PRD look finished.
- ❌ Treating the PRD as authority — it informs tasks; ctl gates them.

<!-- integration:omp -->

Write the PRD under `.ctl/spec/prd/` only if that path is inside the active task's
`write_allow`; otherwise print it and let the user place it. `ctl prd init` is read-only
(prints to stdout). When the PRD is confirmed, route to `ctl-to-tasks` to produce task
proposals; the OMP ctl gate still governs every resulting `ctl task create`.
<!-- integration:opencode -->

Write the PRD under `.ctl/spec/prd/` only if that path is inside the active task's
`write_allow`; otherwise print it for the user to place. `ctl prd init` is read-only.
When the PRD is confirmed, route to `ctl-to-tasks`; the `.opencode/plugins/ctl-gate.ts`
plugin still governs every resulting `ctl task create`.

**Recommended role** (autonomous dispatch — see control-guard): `designer` — PRD
synthesis is authoring a design artifact within scope. Writable role, so it needs an
active in_progress task; hand implementation to `build`.
<!-- integration:claude -->

Write the PRD under `.ctl/spec/prd/` only if that path is inside the active task's `write_allow`; otherwise print it and let the user place it. `ctl prd init` is read-only (prints to stdout). When the PRD is confirmed, route to `ctl-to-tasks`; the Claude Code PreToolUse gate (`.claude/hooks/ctl-gate.py`) still governs every resulting `ctl task create`.
