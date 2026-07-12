---
name: ctl-architecture-review
description: "Periodic read-only architecture review: surface structural deepening candidates and output a candidate report (never code changes). Triggers when: doing a periodic architecture checkup or smelling structural drift. Do NOT trigger for: a refactor already decided (open a task), routine implementation, or debugging (ctl-diagnose)."
---


## The review (phase body)

Begin with the mechanical layer, then the qualitative one.

```
ctl architecture review        # runs every structural check, no fail-fast
ctl architecture review --json # machine-readable {total, passed, failed, checks[]}
```

`ctl architecture review` proves the *registered* invariants (dependency direction, command surface, fixture/gate shape), not depth or locality — that is your job here.

### What to look for (deepening candidates)

- **Shallow modules** — thin wrappers whose interface is nearly as large as their implementation.
- **Concepts spread across too many files** — one idea touched in five places to change once.
- **Poor locality** — related logic far apart; unrelated logic tangled together.
- **Testing seams that hide integration bugs** — mocks that pass while the real wiring is unverified.
- **Hypothetical adapter boundaries** — abstraction built for a second implementation that does not exist.
- **Repeated domain terms without a glossary entry** — same word, drifting meaning, no canonical definition.
- **Duplicated task / run / lease logic** — parallel state machines re-deriving the same rules.
- **Application mega-module risk** — one module accreting unrelated responsibilities.

### Output: a candidate report

For each candidate, record:

| Field | Content |
|---|---|
| candidate | the shallow module / duplication / boundary, named |
| files involved | the concrete files |
| current friction | what is painful or risky today |
| proposed deepening | the structural change that would help |
| expected benefit | what gets simpler / safer |
| testability impact | does it make real integration easier to test? |
| risk | what the change could break |
| contradicts ADR/spec? | does it conflict with an existing decision/spec? |

When the user chooses a candidate, route to `ctl-grill-with-spec` / `ctl-to-tasks` to open a *new* governed task with its own scope and gates.

### Anti-patterns

- ❌ Reporting a verdict ("the architecture is bad") instead of candidates.
- ❌ Proposing a deepening that contradicts an ADR without flagging it in the report.

<!-- integration:omp -->

`ctl architecture review` is read-only (no events). Produce the candidate report as a normal
artifact only if its path is inside an active task's `write_allow`; otherwise print it. A chosen
candidate becomes a NEW governed task (`ctl-grill-with-spec` -> `ctl task create`, gated by the OMP
PreToolUse hook) — this skill never edits code. Pairs with the OMP agent-end architecture-drift
reminder.
<!-- integration:opencode -->

`ctl architecture review` is read-only (no events). Produce the candidate report only if its path
is inside an active task's `write_allow`; otherwise print it. A chosen candidate becomes a NEW
governed task via `ctl task create` (gated by `.opencode/plugins/ctl-gate.ts`) — this skill never
edits code.

**Recommended role** (autonomous dispatch — see control-guard): `explore` — the survey
and candidate report are read-only (always spawnable, no active task required). Authoring
a chosen candidate into a new governed task is `designer`; implementing it is `build`.
<!-- integration:claude -->

`ctl architecture review` is read-only (no events). Produce the candidate report as a normal artifact only if its path is inside an active task's `write_allow`; otherwise print it. A chosen candidate becomes a NEW governed task (`ctl-grill-with-spec` -> `ctl task create`, gated by the Claude Code PreToolUse hook) — this skill never edits code. Read-only review can be dispatched to a subagent; keep the follow-up task's edits inline.
