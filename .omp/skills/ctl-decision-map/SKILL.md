---
name: ctl-decision-map
description: "Maintain a project-level decision map for an effort too big to plan upfront — one that grill found has fog (decisions blocked on frontier work advancing). The map is an index, not a store: it tracks Destination, Frontier (ctl task links), Fog (unresolved decisions), and Out of scope; fog graduates into tasks incrementally via ctl-to-tasks rather than one upfront slice pass. Lives at .ctl/spec/maps/<slug>.md (spec tier, human-writable). Triggers when: grill surfaces fog during alignment on a large effort, or a multi-session effort re-opens and needs re-orientation. Do NOT trigger for: a no-fog effort (go straight to ctl-to-tasks), a single well-scoped task, or anything that fits one session."
---

# ctl-decision-map (OMP)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-decision-map/source.md` by `ctl skills sync`. OMP-specific mechanics live after the core.

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
8. **decision map** — situational, triggered from grill when an effort has
   **fog**: decisions that cannot resolve until frontier work advances. Maintains
   a project-level index (Destination · Frontier · Fog · Out of scope); fog
   graduates into tasks incrementally via `ctl-to-tasks` rather than one upfront
   slice pass. If grill surfaces no fog, skip it — the effort fits one session.

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

## Station contract

- **Upstream**: `ctl-grill-with-spec` — when alignment finds an effort has fog
  (decisions that can't resolve until the frontier advances), it seeds a map here
  instead of converging to a single upfront `ctl-to-tasks` pass.
- **Produces**: a decision map at `.ctl/spec/maps/<slug>.md` (status: `draft` →
  `living` while fog remains → `closed` when Destination is reached and no fog
  remains).
- **Downstream**: graduation — each resolved fog item hands off to `ctl-to-tasks`
  (or `ctl task create` for a trivial slice); the new task ID re-enters Frontier.

## When to build a map — and when not to

Read [`decision-map.md`](../../spec/guides/decision-map.md) first. It defines the
format, sections, and graduation rule.

Build a map **only** when grill surfaces fog. The test: are there decisions that
cannot be made until work on the frontier advances? If yes, the effort is too big
to plan upfront — build a map. If every decision resolves in the alignment
interview, the effort fits one session: **go straight to `ctl-to-tasks`** and skip
the map. A no-fog effort gets no map.

## The map discipline

**The map is an index, not a store.** A decision lives in exactly one place — its
ctl task once it graduates. The map gists and links; it never restates a task's
objective or scope (that is what `ctl board` and the task itself are for).

Four sections (see the guide for the full schema):

- **Destination** — the outcome that must be true when done. One sentence.
- **Frontier** — ctl task IDs ready/active; links only, no restatement.
- **Fog** — decisions blocked on frontier work. Each names: the decision · its
  blocker · graduating `kind` (implementation/research) · AFK/HITL.
- **Out of scope** — ruled beyond the destination. Closed, never graduates.

### Graduation (the core mechanic)

```
fog item  --blocker completes, decision resolves (HITL via grill if needed)-->
          ctl-to-tasks slice (or ctl task create)
          --> new task ID enters Frontier
          --> fog item struck from the map
```

Nothing lingers in two places. Graduating fog clears the patch.

### The per-session loop

1. Open the map; re-read **Destination**.
2. `ctl board` / `ctl next-task` confirm the takeable Frontier; claim or resume one.
3. Work it under normal governance (scope, gates, evidence).
4. New decisions found while working → add to **Fog** with their blocker.
   Ruled-out work → **Out of scope**.
5. A completed frontier task may unblock fog → resolve and graduate.
6. **Close** when Destination is reached and no fog remains.

## Anti-patterns

- ❌ Restating a task's objective/scope in the map instead of linking its ID
  (the map is an index, not a store).
- ❌ Building a map for a no-fog effort that fits one session.
- ❌ Letting a decision live in both the map's Fog *and* a graduated task —
  graduate fully: strike the fog.
- ❌ Treating Out-of-scope as deferred Fog — out-of-scope is ruled out, never
  graduates.
- ❌ Letting the map drift from `ctl board` — the Frontier mirrors the machine
  view; if they disagree, the task ledger is truth, the map follows.
- ❌ Using the map as a substitute for governance — it orients a human; it does
  not replace gates, scope, evidence, or `ctl task` lifecycle.

## Provenance

Inspired by Matt Pocock's `wayfinder` skill (v1.1) — the fog-of-war / frontier /
"map is an index, not a store" framing — adapted to ctl's governed-task model:
the map links into ctl tasks and graduates fog via `ctl-to-tasks`, rather than
managing its own ticket tracker. External skill text is L0 reference material;
this is a ctl-native rewrite, not a vendored control.

## OMP Integration (platform-specific)

The map lives at `.ctl/spec/maps/<slug>.md` (spec tier — writable under the OMP
PreToolUse ctl gate; protected paths remain hard-denied). Seed it from the
confirmed alignment note. Graduation writes (`ctl-to-tasks`, `ctl task create`)
go through normal governance. The map itself is a Markdown working artifact —
mutating it is a spec-tier write, recorded by the gate. Read the Frontier from
`ctl board` / `ctl next-task` rather than recomputing it by hand.
