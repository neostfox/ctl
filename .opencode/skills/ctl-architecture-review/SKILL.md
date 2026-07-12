---
name: ctl-architecture-review
description: "Periodic read-only architecture review: surface structural deepening candidates and output a candidate report (never code changes). Triggers when: doing a periodic architecture checkup or smelling structural drift. Do NOT trigger for: a refactor already decided (open a task), routine implementation, or debugging (ctl-diagnose)."
---

# ctl-architecture-review (opencode)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-architecture-review/source.md` by `ctl skills sync`. opencode-specific mechanics live after the core.

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

## opencode Integration (platform-specific)

`ctl architecture review` is read-only (no events). Produce the candidate report only if its path
is inside an active task's `write_allow`; otherwise print it. A chosen candidate becomes a NEW
governed task via `ctl task create` (gated by `.opencode/plugins/ctl-gate.ts`) — this skill never
edits code.

**Recommended role** (autonomous dispatch — see control-guard): `explore` — the survey
and candidate report are read-only (always spawnable, no active task required). Authoring
a chosen candidate into a new governed task is `designer`; implementing it is `build`.
