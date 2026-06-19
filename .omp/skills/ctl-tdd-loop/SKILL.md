---
name: ctl-tdd-loop
description: "Drive implementation as a strict TDD loop: one behavior, one red-capable test, captured failure evidence, minimal implementation, captured green evidence, refactor only after green. Tests public behavior, not private detail; never claims TDD without red evidence. Triggers when: implementing a behavior change under a task, especially one opted into the ctl tdd-red-green interlock. Do NOT trigger for: pure refactors with no behavior change, docs, or diagnosis (ctl-diagnose)."
---

# ctl-tdd-loop (OMP)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-tdd-loop/source.md` by `ctl skills sync`. OMP-specific mechanics live after the core.

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

## The loop (phase body)

1. **Pick one behavior** — the smallest observable change in public behavior.
2. **Write one red-capable test** for it — a test that *can* fail and currently
   describes behavior that does not yet exist.
3. **Run it and capture the failure** as ctl evidence:
   `ctl gate run --id <id> --gate cargo_test` records the FAIL on the ledger.
4. **Implement the minimal change** that makes it pass — nothing speculative.
5. **Run it and capture the pass**: `ctl gate run --id <id> --gate cargo_test`
   records the PASS. The FAIL-before-PASS history is the red→green proof the
   interlock checks.
6. **Refactor only after green**, with the test still passing.
7. **Repeat** for the next behavior.

### Why the evidence matters

A task opted into TDD (`ctl task create --tdd ... --gates cargo_test`) cannot
`finish` unless the recorded `cargo_test` history contains a failing result at an
earlier seq than a passing one. So "I did TDD" is not an assertion — it is gate
evidence on the ledger. Skip the red capture and the interlock blocks finish.

### Forbidden

- Writing a large batch of speculative tests up front.
- Modifying a test to fit the implementation **without recording why** (the change
  must be justified in the task record, not silent).
- Testing private implementation detail when public behavior can be tested.
- Claiming TDD with no red evidence — the interlock exists precisely to catch this.

### If ctl evidence can't capture red/green

If a particular gate can't represent the red/green run, write the loop's evidence
into the task artifact and note the gap; do not pretend the interlock passed.

### Anti-patterns

- ❌ Green-only history (test only ever passed) on a TDD-opted task.
- ❌ One giant test covering five behaviors.
- ❌ Asserting on internals instead of observable behavior.
- ❌ Editing the test to match a bug, silently.

## OMP Integration (platform-specific)

Opt a task in at create time: `ctl task create --tdd ... --gates cargo_test` (sugar for the
`tdd-red-green` risk trigger). Capture each run with `ctl gate run --id <id> --gate
cargo_test`; the OMP gate governs the edits between runs. `ctl task finish` enforces the
red→green interlock — there is no flag to skip it.
