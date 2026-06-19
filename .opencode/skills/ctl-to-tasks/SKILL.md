---
name: ctl-to-tasks
description: "Convert a confirmed PRD or plan into ctl task proposals — vertical slices, each independently verifiable, each declaring objective, read scope, write scope, gates, acceptance evidence, an AFK/HITL label, and blocking uncertainties. Triggers when: a PRD or plan exists and you need to break it into governed tasks. Do NOT trigger for: scoping a single fuzzy request (ctl-brainstorm), or fabricating task state without a plan. Never creates tasks that bypass protected-path controls and never synthesizes completed ctl events from a plan."
---

# ctl-to-tasks (opencode)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-to-tasks/source.md` by `ctl skills sync`. opencode-specific mechanics live after the core.

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

## Decompose into tasks (phase body)

Prefer **vertical slices** over horizontal layers. Each proposal must declare:

| Field | Rule |
|---|---|
| objective | one testable sentence |
| read scope | what the task may read |
| write scope | the **minimal** `write_allow` (start narrow, widen only on approval) |
| gates | required gate templates (`cargo_fmt_check`, `cargo_check`, `cargo_clippy`, `cargo_test`) |
| acceptance evidence | the artifact that proves done (test output, run output) |
| AFK / HITL | can it run unattended, or does it need a human in the loop? |
| blocking uncertainties | the OpenUncertainty items that must resolve first |

**AFK vs HITL.** Label each task: **AFK** (away-from-keyboard — deterministic,
low-risk, fully gated, safe to run unattended) or **HITL** (human-in-the-loop —
needs a decision, touches a protected path via exception, or carries unresolved
risk). When unsure, label HITL.

### Hard rules

- Prefer vertical slices; split anything too big to finish inside one boundary.
- Each task is independently verifiable on its own evidence.
- Non-overlapping `write_allow` across sibling tasks (overlap forces sequencing).
- **Do not create tasks that bypass protected-path controls** (`.git/`,
  `events.jsonl`, `schemas/`, `Cargo.toml`).
- **Do not synthesize completed ctl events from a PRD/plan.** A plan describes
  intended work; only real execution + evidence may close a task.

### Output: proposals, then governed creation

Write proposals to `.ctl/tasks/<task-id>/proposal.md` (within scope), or dry-run
the boundary before committing to it:

```
ctl task create --dry-run --id <id> --objective "..." \
  --read-scope <p> --write-allow <p> --gates cargo_check --gates cargo_test
```

`--dry-run` validates and shows what would happen without persisting — a safe way
to check a proposed boundary before the real create. Hand approved proposals to
control-guard for the actual `ctl task create`.

### Anti-patterns

- ❌ Horizontal layer tasks that can't be verified alone.
- ❌ A broad `write_allow` "to be safe".
- ❌ Overlapping write scopes across sibling tasks.
- ❌ Marking a risky task AFK to avoid asking.
- ❌ Emitting events that claim work is done before it ran.

## opencode Integration (platform-specific)

Proposals are notes; the real `ctl task create` is gated by `.opencode/plugins/ctl-gate.ts`.
Use `ctl task create --dry-run` to preview a boundary and `ctl board` to check sibling
tasks for write-scope overlap before creating. Record PRD provenance with `ctl brainstorm`.

**Recommended role** (autonomous dispatch — see control-guard): `designer` — shaping
vertical task proposals from a PRD is design work; the proposals are notes until the
gated `ctl task create`. Writable role, so it needs an active in_progress task.
