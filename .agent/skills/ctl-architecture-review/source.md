---
name: ctl-architecture-review
description: "Periodic read-only architecture review: start from `ctl architecture review` (the mechanical structural checks), then surface deepening candidates — shallow modules, concepts spread across too many files, poor locality, testing seams that hide integration bugs, hypothetical (not real) adapter boundaries, duplicated task/run/lease logic, application mega-module risk, and repeated domain terms with no glossary entry. Outputs a candidate report, never code changes; the user chooses which candidate becomes a new governed task. Triggers when: doing a periodic architecture checkup or smelling structural drift. Do NOT trigger for: a refactor already decided (open a task), routine implementation, or debugging (ctl-diagnose)."
---


## The review (phase body)

Begin with the mechanical layer, then the qualitative one.

```
ctl architecture review        # read-only: runs every structural check, no fail-fast
ctl architecture review --json # machine-readable {total, passed, failed, checks[]}
```

`ctl architecture review` is **read-only** (emits no events). It proves the
*registered* invariants (dependency direction, command surface, fixture/gate
shape). It does **not** judge depth or locality — that is your job here.

### What to look for (deepening candidates)

- **Shallow modules** — thin wrappers whose interface is nearly as large as their
  implementation; little hidden, much surface.
- **Concepts spread across too many files** — one idea you must touch in five
  places to change once.
- **Poor locality** — related logic far apart; unrelated logic tangled together.
- **Testing seams that hide real integration bugs** — mocks/abstractions that make
  tests pass while the real wiring is unverified.
- **Hypothetical (not real) adapter boundaries** — abstraction built for a second
  implementation that does not exist.
- **Repeated domain terms without a glossary entry** — the same word used with
  drifting meaning and no canonical definition.
- **Duplicated task / run / lease logic** — parallel state machines re-deriving the
  same rules.
- **Application mega-module risk** — one module accreting unrelated
  responsibilities.

### Output: a candidate report (no changes)

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

### Hard rule

**No code changes.** This skill is read-only by default. When the user chooses a
candidate, route to `ctl-brainstorm` / `ctl-to-tasks` to open a *new* governed
task with its own scope and gates — the review never edits code itself.

### Anti-patterns

- ❌ Editing code "while you're in there".
- ❌ Reporting a verdict ("the architecture is bad") instead of candidates.
- ❌ Proposing a deepening that contradicts an ADR without flagging it.
- ❌ Treating `ctl architecture review` PASS as proof the design is deep — it only
  proves the registered structural invariants hold.

<!-- integration:omp -->

`ctl architecture review` is read-only (no events). Produce the candidate report as a normal
artifact only if its path is inside an active task's `write_allow`; otherwise print it. A chosen
candidate becomes a NEW governed task (`ctl-brainstorm` -> `ctl task create`, gated by the OMP
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

`ctl architecture review` is read-only (no events). Produce the candidate report as a normal artifact only if its path is inside an active task's `write_allow`; otherwise print it. A chosen candidate becomes a NEW governed task (`ctl-brainstorm` -> `ctl task create`, gated by the Claude Code PreToolUse hook) — this skill never edits code. Read-only review can be dispatched to a subagent; keep the follow-up task's edits inline.
