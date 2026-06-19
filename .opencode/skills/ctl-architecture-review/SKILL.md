---
name: ctl-architecture-review
description: "Periodic read-only architecture review: start from `ctl architecture review` (the mechanical structural checks), then surface deepening candidates — shallow modules, concepts spread across too many files, poor locality, testing seams that hide integration bugs, hypothetical (not real) adapter boundaries, duplicated task/run/lease logic, application mega-module risk, and repeated domain terms with no glossary entry. Outputs a candidate report, never code changes; the user chooses which candidate becomes a new governed task. Triggers when: doing a periodic architecture checkup or smelling structural drift. Do NOT trigger for: a refactor already decided (open a task), routine implementation, or debugging (ctl-diagnose)."
---

# ctl-architecture-review (opencode)

Review architecture **read-only** and hand back candidates, not changes. Start from the mechanical
compliance check, then add the qualitative judgement a checker can't make. You never refactor here:
a chosen candidate becomes a *new* governed task.

The **managed core** below is the platform-neutral ctl workflow protocol,
byte-checked by CI against `.agent/protocols/workflow-skills.md` and the OMP
copy. Do not edit it here in isolation. opencode-specific mechanics live after the core.

<!-- ctl:workflow-core:start version=1 -->
# ctl Workflow Skills — Core Protocol

WORKFLOW_PROTOCOL_VERSION = 1

This is the platform-neutral workflow-skills core. It is embedded **verbatim**
inside every ctl workflow skill's managed-core block; the canonical copy lives at
`.agent/protocols/workflow-skills.md`. A CI drift check fails if any copy (or its
declared version) diverges. Edit this file and re-sync every workflow skill
together — never one in isolation. Nothing platform-specific (tool names, hook
mechanics, plugin paths) and nothing phase-specific belongs here; that lives in
each skill outside the managed core.

## Division of labor (non-negotiable)

Skills and agents manage **semantic workflow** — what to think about, in what
order, and which artifact each phase produces. ctl manages **facts, scope,
evidence, gates, ledgers, and honest disclosure**. A workflow skill never relaxes
a boundary, never declares a task complete, and never substitutes its own
judgement for ctl evidence. Workflow discipline is not proof: it does not replace
gates, audits, reviewer independence, or tamper evidence, and it never creates a
verdict.

## Phase map

Phases run in this order; skip any whose preconditions are already met. Each
phase is a separate skill that carries this same core plus its own body.

1. **grill / first principles** — before a PRD or implementation, when the
   request is ambiguous, too broad, high-risk, or likely to build the wrong
   thing. Output: alignment artifacts (observed facts, declared rules,
   assumptions, irreducible constraints, goals, non-goals, unknowns, a minimum
   viable experiment) — not a claim of truth.
2. **PRD** — after enough context is resolved and before generating multiple
   durable tasks. Draft until explicitly confirmed. Separate **ObservedBasis**
   (what the agent actually read) from **ConfirmedBasis** (what a user or project
   authority confirmed) from **OpenUncertainty** (unresolved unknowns, never
   hidden). Status is one of: draft · confirmed · superseded.
3. **tasks / issues** — vertical slices, each independently verifiable, each
   declaring objective, read scope, write scope, gates, acceptance evidence, and
   an **AFK / HITL** label and blocking uncertainties. Never bypass protected-path
   controls; never synthesize completed ctl events from a plan.
4. **TDD** — during implementation: one behavior at a time, red evidence before
   green evidence, refactor only after green. Test public behavior, not private
   implementation detail.
5. **diagnose / bayesian** — for bugs, flaky behavior, unexpected results, or
   architectural uncertainty. No fix before a red-capable feedback loop.
   Hypotheses must be falsifiable; prefer discriminating evidence over broad
   speculation.
6. **architecture review** — read-only by default; outputs candidates, not code
   changes. The user chooses which candidate becomes a new governed task.
7. **handoff** — when the session is long, context is high, when switching
   agents or platforms, or before AFK / starting a separate governed run.

## Thinking frameworks, placed

- **First Principles** belongs in grill / design clarification: restate the
  problem, separate observed facts from declared rules from assumptions, name the
  irreducible constraints, and challenge inherited ones — "are we doing this
  because the domain requires it, or because the existing architecture/framework
  suggests it?" Outputs are artifacts, not verdicts.
- **Bayesian reasoning** belongs in diagnose / loop-breaking: rank falsifiable
  hypotheses, grade evidence, update belief in plain language, and seek the
  discriminating test. Ranked hypotheses and evidence quality — not numeric
  confidence scoring.

Do not create floating, generic "think better" skills; the frameworks live
inside the phases above.

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

## Provenance

These workflow skills are ctl-native rewrites inspired by Matt Pocock's
engineering skill workflow and by the First Principles / Bayesian placement
introduced in Trellis PR #335. External skill text is treated as L0 reference
material; ctl does not vendor third-party skills as an active control plane and
does not place them inside its trust boundary.
<!-- ctl:workflow-core:end -->

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

## opencode Integration (platform-specific)

`ctl architecture review` is read-only (no events). Produce the candidate report only if its path
is inside an active task's `write_allow`; otherwise print it. A chosen candidate becomes a NEW
governed task via `ctl task create` (gated by `.opencode/plugins/ctl-gate.ts`) — this skill never
edits code.
