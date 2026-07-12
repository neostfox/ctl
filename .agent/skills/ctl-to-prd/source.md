---
name: ctl-to-prd
description: "Synthesize resolved context into a PRD for ctl-to-tasks. Triggers when: enough context is resolved (often after ctl-grill-with-spec) and you are about to spin up multiple durable tasks. Do NOT trigger for: a single obvious task (go straight to ctl-to-tasks), or when key intent is still unknown (grill first)."
---


## Station contract

- **Produces**: a PRD at `.ctl/spec/prd/<prd-id>.md` (status: draft → confirmed).
- **Downstream**: `ctl-to-tasks` consumes the **confirmed** PRD.

## Synthesize the PRD (phase body)

From a **confirmed** alignment note (`ctl-grill-with-spec`); if none exists and the request is non-trivial, grill first. Scaffold, then fill from resolved context — no re-grilling:

```
ctl prd init --title "<title>" > .ctl/spec/prd/<prd-id>.md
```

`ctl prd init` prints a template (Objective / Context / Tasks). The `## Tasks` section — vertical, independently shippable slices — is the hand-off to `ctl-to-tasks`. V1 is an agent-readable artifact workflow only: no PRD subsystem, no events; the PRD never gates a task.

### The three bases (never collapse them)

Every claim in the PRD is tagged by where it came from:

- **ObservedBasis** — what the agent actually read or ran (cite the file/command).
- **ConfirmedBasis** — what the user or an existing project authority explicitly confirmed.
- **OpenUncertainty** — unresolved unknowns, surfaced not hidden; they travel into task proposals as blocking uncertainties.

A belief with no observation and no confirmation is OpenUncertainty, not a requirement.

### Status lifecycle

The PRD header carries exactly one status: `draft` (synthesized, not yet confirmed) → `confirmed` (user accepted as the basis for task generation) → `superseded` (replaced; link forward). Stay in `draft` until explicitly confirmed; do not generate durable tasks from a `draft` PRD unless the user asks for a dry run.

### Anti-patterns

- ❌ Re-interviewing the user for context already resolved in the grill.

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
