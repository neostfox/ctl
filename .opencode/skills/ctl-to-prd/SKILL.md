---
name: ctl-to-prd
description: "Synthesize the current resolved context into a PRD — do NOT re-interview the user unless information is genuinely missing. Separates ObservedBasis (what the agent read) from ConfirmedBasis (what an authority confirmed) from OpenUncertainty (unresolved unknowns, never hidden), and carries a draft/confirmed/superseded status. Triggers when: enough context is resolved (often after ctl-grill-with-spec) and you are about to spin up multiple durable tasks. Do NOT trigger for: a single obvious task (go straight to ctl-to-tasks), or when key intent is still unknown (grill first)."
---

# ctl-to-prd (opencode)

Turn resolved context into a Product Requirements Document. You are **synthesizing what is
already known**, not restarting discovery: re-interview the user only for information that
is genuinely missing. The PRD is the durable bridge between alignment (grill) and a set of
vertical tasks (`ctl-to-tasks`).

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

## Synthesize the PRD (phase body)

Scaffold the shape with the ctl CLI, then fill it from resolved context:

```
ctl prd init --title "<title>" > .ctl/spec/prd/<prd-id>.md
```

`ctl prd init` prints a structured PRD template (Objective / Context / Tasks).
Filling it is the "grill" step; the `## Tasks` section is the hand-off to
`ctl-to-tasks`. V1 is an **agent-readable artifact workflow only** — there is no
PRD subsystem, no PRD events, and the PRD never gates a task.

### The three bases (never collapse them)

Every claim in the PRD is tagged by where it came from:

- **ObservedBasis** — what the agent actually read or ran (cite the file/command).
- **ConfirmedBasis** — what the user or an existing project authority explicitly
  confirmed.
- **OpenUncertainty** — unresolved unknowns. These must be surfaced, never hidden;
  they travel into the task proposals as blocking uncertainties.

A belief with no observation and no confirmation is OpenUncertainty, not a
requirement.

### Status lifecycle

The PRD header carries exactly one status:

- `draft` — synthesized but not yet confirmed by the user/authority.
- `confirmed` — the user accepted it as the basis for task generation.
- `superseded` — replaced by a later PRD (link forward).

Stay in `draft` until explicitly confirmed. Do not generate durable tasks from a
PRD that is still `draft` unless the user asks for a dry run.

### Quality bar

- Every requirement is tagged ObservedBasis / ConfirmedBasis / OpenUncertainty.
- No OpenUncertainty was silently promoted into a requirement.
- The `## Tasks` section lists vertical, independently shippable slices.
- Status is set honestly; a draft is labelled a draft.

### Anti-patterns

- ❌ Re-interviewing the user for context already resolved in the grill.
- ❌ Presenting an assumption as a confirmed requirement.
- ❌ Hiding an unknown to make the PRD look finished.
- ❌ Treating the PRD as authority — it informs tasks; ctl gates them.

## opencode Integration (platform-specific)

Write the PRD under `.ctl/spec/prd/` only if that path is inside the active task's
`write_allow`; otherwise print it for the user to place. `ctl prd init` is read-only.
When the PRD is confirmed, route to `ctl-to-tasks`; the `.opencode/plugins/ctl-gate.ts`
plugin still governs every resulting `ctl task create`.

**Recommended role** (autonomous dispatch — see control-guard): `designer` — PRD
synthesis is authoring a design artifact within scope. Writable role, so it needs an
active in_progress task; hand implementation to `build`.
