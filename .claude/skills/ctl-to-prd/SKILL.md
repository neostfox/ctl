---
name: ctl-to-prd
description: "Synthesize resolved context into a PRD for ctl-to-tasks. Triggers when: enough context is resolved (often after ctl-grill-with-spec) and you are about to spin up multiple durable tasks. Do NOT trigger for: a single obvious task (go straight to ctl-to-tasks), or when key intent is still unknown (grill first)."
---

# ctl-to-prd (Claude Code)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-to-prd/source.md` by `ctl skills sync`. Claude Code-specific mechanics live after the core.

<!-- ctl:workflow-core:start version=1 -->
# ctl Workflow Skills — Core Protocol

WORKFLOW_PROTOCOL_VERSION = 1

This is the platform-neutral workflow-skills core. It is split into an
**embedded** part (division of labor + invariants), carried verbatim inside
every workflow skill's managed-core block, and a **reference** part (phase map +
frameworks + provenance) that lives only in this file — the auto-loaded
control-guard carries the pipeline routing, and each skill's body covers its own
phase. The canonical copy lives at `.agent/protocols/workflow-skills.md`; a CI
drift check fails if any embedded copy diverges. Edit this file and re-sync
every workflow skill together — never one in isolation. Nothing platform-specific
(tool names, hook mechanics, plugin paths) and nothing phase-specific belongs in
the embedded part; that lives in each skill outside the managed core.

## Division of labor (non-negotiable)

Skills and agents manage **semantic workflow** — what to think about, in what
order, and which artifact each phase produces. ctl manages **facts, scope,
evidence, gates, ledgers, and honest disclosure**. A workflow skill never relaxes
a boundary, never declares a task complete, and never substitutes its own
judgement for ctl evidence. Workflow discipline is not proof: it does not replace
gates, audits, reviewer independence, or tamper evidence, and it never creates a
verdict.

## Invariants every phase honors

- Produce **artifacts, not claims**. "Done" is an evidence artifact ctl can see,
  never an assertion — "where is the evidence?"
- Keep **draft separate from confirmed basis**; disclose open uncertainty rather
  than hiding it.
- **Red before green**: no green claim without prior red evidence for the same
  behavior.
- **No fix before a reproduction loop.**
- **Architecture review is read-only**; a refactor needs a fresh governed task.
- External workflow inspiration is **L0 reference material** (see Provenance) —
  never an authority, never vendored as an active control.
<!-- ctl:workflow-core:end -->

*The phase map, frameworks, and provenance are reference material in `.agent/protocols/workflow-skills.md` — not embedded here. The auto-loaded control-guard carries the pipeline routing; this skill's body covers its own phase.*

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

## Claude Code Integration (platform-specific)

Write the PRD under `.ctl/spec/prd/` only if that path is inside the active task's `write_allow`; otherwise print it and let the user place it. `ctl prd init` is read-only (prints to stdout). When the PRD is confirmed, route to `ctl-to-tasks`; the Claude Code PreToolUse gate (`.claude/hooks/ctl-gate.py`) still governs every resulting `ctl task create`.
