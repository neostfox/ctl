---
name: ctl-handoff
description: "Compact the current task/session context into a portable handoff another agent or human can safely pick up. Triggers when: the session is long, context is high, switching agents/platforms, before AFK, or before a separate governed run. Do NOT trigger for: a quick in-session continuation."
---


## Handoff (phase body)

Start from the ledger-derived artifact, then layer on judgement:

```
ctl handoff export --id <task> [--json]
```

`ctl handoff export` is strictly read-only (appends nothing) and emits a `control.handoff.v1` snapshot: objective + boundary, per-gate status, the completion-interlock verdict, the drift-derived next action, the uncommitted files inside the task's write scope, and the recent event tail.

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
- **Dispatch binding** — whether `CTL_TASK_ID` (or equivalent) is active, so the next agent governs its calls by the right task.

### No active task?

If there is no active task, produce a conversation/project handoff **only** if a safe in-scope location allows it; otherwise **print the handoff to stdout**. Never invent a write target to place a handoff.

### Anti-patterns

- ❌ A handoff that logs what was done but not the decisions and dead ends behind it.
- ❌ Omitting the active dispatch binding, so the next agent misbinds to the wrong task.

<!-- integration:omp -->

`ctl handoff export` is read-only and always safe to run. Persist the enriched handoff to
`.ctl/tasks/<task-id>/handoff.md` only if that path is in the active task's `write_allow`;
otherwise print it. Note whether `CTL_TASK_ID` is set so the next OMP session binds to the
right task. Pairs with the OMP session-shutdown unfinished-task reminder.
<!-- integration:opencode -->

`ctl handoff export` is read-only and always safe to run. Persist the enriched handoff to
`.ctl/tasks/<task-id>/handoff.md` only if that path is in the active task's `write_allow`;
otherwise print it. Note whether `CTL_TASK_ID` is set so the next opencode session binds to
the right task (the plugin reads it to resolve multi-active ambiguity).

**Recommended role** (autonomous dispatch — see control-guard): `explore` — `ctl handoff
export` and summarizing prior context are read-only (always spawnable). Persisting the
handoff inside scope is a write the dispatching task already governs.
<!-- integration:claude -->

`ctl handoff export` is read-only and always safe to run. Persist the enriched handoff to `.ctl/tasks/<task-id>/handoff.md` only if that path is in the active task's `write_allow`; otherwise print it. Note whether `CTL_TASK_ID` is set so the next Claude Code session binds to the right task (the `.claude/hooks/ctl-gate.py` gate reads it to resolve multi-active ambiguity).
