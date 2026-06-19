---
name: ctl-grill-with-spec
description: "Align before building: grill an ambiguous, broad, or high-risk request from first principles, separating observed facts from declared rules from assumptions, naming irreducible constraints and a minimum viable experiment. Produces alignment artifacts (and, only when scope allows and the user confirms, a domain/ADR note) — never a claim of truth. Triggers when: the request is vague, too broad, high-risk, or likely to produce the wrong thing, before a PRD or implementation. Do NOT trigger for: an already well-scoped request (go to ctl-to-prd or ctl-to-tasks), code review (ctl-review), or debugging (ctl-diagnose)."
---

# ctl-grill-with-spec (opencode)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-grill-with-spec/source.md` by `ctl skills sync`. opencode-specific mechanics live after the core.

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

## The grill (first-principles phase body)

Evidence before questions: anything the repository can answer, read — code,
tests, configs, specs, task history. Only ask the user for what the repo cannot
answer (intent, preference, risk tolerance). Then assemble the alignment artifact:

| Field | What it captures |
|---|---|
| Observed facts | what you actually read or ran (cite the source) |
| Declared rules | invariants the project states (specs, schemas, guides) |
| Assumptions | beliefs you are carrying that are not yet confirmed |
| Irreducible constraints | what cannot change (domain, physics, contracts) |
| User goals | the outcome that must be true when done |
| Non-goals | what is explicitly out of scope |
| Unknowns | unresolved questions, ranked by how much they could change scope |
| Minimum viable experiment | the smallest probe that would confirm direction |

**Challenge inherited assumptions.** For each assumption ask: *are we doing this
because the domain requires it, or because the existing architecture/framework
suggests it?* Strike or downgrade anything that is convention masquerading as a
constraint.

**Outputs are artifacts, not truth.** A grill records what you currently believe
and why — it never asserts the answer is correct. The next phase (`ctl-to-prd`)
turns confirmed alignment into a PRD; unconfirmed items travel forward as
OpenUncertainty, never silently resolved.

### Where artifacts go (only within scope, only when confirmed)

- Working notes: `.ctl/tasks/<task-id>/grill.md` (inside the active task's
  `write_allow`).
- A crystallized domain term or decision **only when the user confirms it**:
  `.ctl/spec/domain.md` or `.ctl/spec/adr/ADR-xxxx.md`. Do **not** write a
  domain/ADR doc on your own judgement or outside the task's write scope — an ADR
  records a *confirmed* decision, not a draft thought.

### Quality bar

- Every "fact" cites where it came from; unconfirmed beliefs are labelled
  assumptions, not facts.
- At least one inherited assumption was challenged and resolved (kept / struck).
- Non-goals are explicit, not implied.
- Unknowns are disclosed, not buried; the riskiest one names the experiment that
  would settle it.

### Anti-patterns

- ❌ Asking the user something the repository already answers.
- ❌ Presenting an assumption as an observed fact.
- ❌ Writing a domain/ADR doc without user confirmation or outside write scope.
- ❌ Treating the grill's conclusions as proven rather than as artifacts.

## opencode Integration (platform-specific)

Invoke during scoping, alongside `ctl-brainstorm`. Record the cognitive artifacts the
eventual task derived from with `ctl brainstorm` provenance (record-only — never gates,
no quality claim). Writing `grill.md` or an ADR is a mutating write gated by
`.opencode/plugins/ctl-gate.ts`: it must fall inside the active task's `write_allow` or
the plugin throws. Hand confirmed alignment to `ctl-to-prd`; a durable lesson to
`ctl-spec-update`.

**Recommended role** (autonomous dispatch — see control-guard): `explore` for the
read-only investigation and alignment; `designer` when authoring `grill.md` or an ADR
inside an active task's scope. `explore` is the only read-only role.
