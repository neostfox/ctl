---
name: ctl-handoff
description: "Compact the current task/session context into a portable handoff another agent or human can safely pick up: task, phase, objective, decisions, modified files, gates run, open uncertainties, next safe action, do-not-do list, environment hazards, adapter/platform, and whether a dispatch binding (CTL_TASK_ID) is active. Triggers when: the session is long, context is high, switching agents/platforms, before AFK, or before a separate governed run. Do NOT trigger for: a quick in-session continuation."
---

# ctl-handoff (OMP)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-handoff/source.md` by `ctl skills sync`. OMP-specific mechanics live after the core.

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

## Handoff (phase body)

Start from the ledger-derived artifact, then layer on judgement:

```
ctl handoff export --id <task> [--json]
```

`ctl handoff export` is strictly read-only (appends nothing) and emits a
`control.handoff.v1` snapshot: objective + boundary, per-gate status, the
completion-interlock verdict, the drift-derived next action, the uncommitted files
inside the task's write scope, and the recent event tail.

### Add what the export can't know

The export gives facts; you add the **decisions and hazards**:

- **Current task / phase / objective** — from the export.
- **Decisions made** — and the reasoning, so they aren't relitigated.
- **Modified files** — the export lists uncommitted in-scope files; note *why*.
- **Gates run** — from the export's per-gate status.
- **Open uncertainties** — what is still unresolved (carry forward, don't bury).
- **Next safe action** — the single safest next step.
- **Do-not-do list** — dead ends, things tried, things explicitly out of scope.
- **Known local environment hazards** — OS quirks, stale binaries, flaky steps.
- **Adapter / platform used** — OMP, opencode, manual.
- **Dispatch binding** — whether `CTL_TASK_ID` (or equivalent) is active, so the
  next agent governs its calls by the right task.

### No active task?

If there is no active task, produce a conversation/project handoff **only** if a
safe in-scope location allows it; otherwise **print the handoff to stdout**. Never
invent a write target to place a handoff.

### Quality bar

- A fresh agent could resume without re-deriving the decisions.
- The do-not-do list captures the dead ends, not just the successes.
- Open uncertainties are disclosed, not smoothed over.
- The next safe action is a single concrete step.

### Anti-patterns

- ❌ A handoff that lists what was done but not what was decided or avoided.
- ❌ Hiding an unresolved uncertainty to look finished.
- ❌ Writing the handoff outside the active task's write scope.
- ❌ Omitting the active dispatch binding, so the next agent misbinds.

## OMP Integration (platform-specific)

`ctl handoff export` is read-only and always safe to run. Persist the enriched handoff to
`.ctl/tasks/<task-id>/handoff.md` only if that path is in the active task's `write_allow`;
otherwise print it. Note whether `CTL_TASK_ID` is set so the next OMP session binds to the
right task. Pairs with the OMP session-shutdown unfinished-task reminder.
