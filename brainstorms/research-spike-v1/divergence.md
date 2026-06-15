# Divergence — Research/Spike V1 (BS-RS1)

> Originator artifact. L0 content. Task: `research-spike-v1`.
> Records the candidate directions before convergence. Leans are not decisions;
> convergence (post-critic) decides.

## Problem

`ctl` only models implementation work: a task succeeds by producing code that
passes gates. But the highest-value early work on a fuzzy idea is often a
*spike* — produce evidence, update the uncertainty map, recommend a design —
and produce little or no code. Today such a task either looks like a failure
(no code) or is forced into an implementation shape. Research/Spike V1 adds a
**task kind** whose completion is defined by *evidence + epistemic outcomes*,
not by code. It must do this without re-introducing the things `EPISTEMIC_
CONTROL.md` §6/§9 forbid: no aggregate verdict, no score, and — critically —
**no "fewer open unknowns" success metric** (a spike that opens four hidden
risks is a success).

This builds directly on the shipped Uncertainty Ledger: the epistemic outcomes
already live there, so this feature must *reuse* them, not duplicate them.

## Axis A — `task_kind` representation & mutability

- **A1.** Enum `TaskKind { Implementation, Research }`, field on `TaskState`,
  set at `task_created`, `#[serde(default)]` → `Implementation` (legacy & absent).
- **A2.** A free-form string tag.
- **A3.** Mutable via `task revise` vs immutable after create.

Lean: **A1**, **immutable after create**. An enum with a defaulting field replays
old streams unchanged (same pattern as `brainstorm_ref` / `uncertainties`).
Immutability matters for integrity: if kind were revisable, an implementer could
flip `research → implementation` at the last moment to dodge the artifact
requirement, or `implementation → research` to dodge code review. Kind is part
of the task's identity, fixed when it is created. (Open Q for critic: is
immutability too rigid — is there a legitimate reclassification case?)

## Axis B — completion enforcement & where it lives

- **B1.** In `finish_task`'s interlock (it already loads the event stream),
  gated on `state.task_kind == Research`: require ≥1 `research_artifact_recorded`
  AND ≥1 uncertainty outcome (a recorded uncertainty, or a disposition).
- **B2.** A separate `research finish` command / separate interlock path.

Lean: **B1.** One interlock, one finish command; research adds two structural
checks *after* the normal integrity checks (tree/policy/gates/audit all still
apply — a spike is not exempt from execution integrity). Reusing `finish_task`
keeps the lifecycle uniform.

What counts as an "uncertainty outcome" (per the directive): a new uncertainty,
or a disposition (resolved / accepted_as_assumption / invalidated). **Merely
recording one open uncertainty satisfies the rule** — by design, because opening
unknowns is a legitimate spike result. (Open Q for critic: is "≥1 artifact + ≥1
any-uncertainty" too low a bar — trivially satisfiable by one empty findings.md
plus one open unknown? Is that acceptable for V1, given §9 forbids a quality
judgment anyway?)

## Axis C — deriving "new uncertainties discovered during task"

- **C1.** Derive from the event stream: count `uncertainty_recorded` events whose
  `seq` > the task's first `task_started` seq. Never persist a counter.
- **C2.** Persist a discovered-count on `TaskState`.

Lean: **C1**, using the **first** `task_started` seq. §"How to calculate" is
explicit: derive from sequence boundaries, do not persist a mutable counter
(mirrors the lazy-derivation discipline of staleness and drift). "Opened" =
*all* uncertainties recorded on the task; "discovered during task" = the subset
recorded after work began (a task may record uncertainties while still in
Planning, before start). (Open Q for critic: with reopen producing multiple
`task_started` events, which boundary — first start, or latest? First start is
the honest "when the work began"; latest would let a reopen reset the count.)

## Axis D — event vocabulary

- **D1.** One new event: `research_artifact_recorded { artifact_ref, artifact_kind,
  source_run_id?, trust_level=content_l0 }`. No `research_completed` event.
- **D2.** Add a `research_completed` summary event.

Lean: **D1.** The epistemic outcomes are already canonical (`uncertainty_*`
events); a `research_completed` event would duplicate them and, worse, read as
"research was sufficiently done" — the same compression `EPISTEMIC_CONTROL.md`
§7 rejects for `brainstorm_completed`. The research output is *derived* at read
time from the artifact + uncertainty events, never snapshotted.

## Axis E — `artifact_kind`

- **E1.** Fixed enum: `findings | experiment | recommendation | design_draft`;
  reducer rejects anything else.
- **E2.** Free-text string.

Lean: **E1.** A fixed enum mirrors the disposition-enum discipline and keeps the
disclosure legible; the four values cover V1's stated cases. (Open Q for critic:
is a fixed enum too rigid for the open-ended nature of research — should it be a
free string, or the enum plus an `other`?)

## Axis F — artifact handling & trust

- **F1.** Reuse `ArtifactRef { path, hash }`, hash computed ctl-side from a
  normalized path (same primitive as evidence_ref). Pin `trust_level=content_l0`;
  disclose `attestation: unavailable`. Suggested locations `research/<id>/`,
  `design/<id>/` — but the boundary, not a hardcoded prefix, governs where writes
  land.

Lean: **F1**, hard. No new trust model (§9). A research artifact is L0 content
exactly like a brainstorm artifact; recording its hash binds *what* was produced,
never asserts the findings are correct.

## Axis G — disclosure surface

- **G1.** A `RESEARCH OUTPUT` block (raw facts: artifacts produced; uncertainties
  opened / resolved-with-evidence / assumptions / invalidated; new discovered
  during task; then artifact + uncertainty refs). Shown for `task_kind=research`
  in `task status` and at completion. No verdict, no score, no ratio.

Lean: **G1.** Implementation tasks never show it. Counts are raw; "new discovered"
is disclosed as a neutral fact, never colored as good or bad.

## Explicitly out of scope (carried from directive)

Full research-methodology engine · claim ontology · experiment runner · automatic
web research · requirement coverage · fewer-unknowns success metric · whole-
brainstorm skip path · U-2 evidence-externality enforcement · principal
authentication · any aggregate research verdict/score.

## Open questions for the critic

1. Is "≥1 artifact + ≥1 uncertainty outcome" a meaningful completion bar, or
   trivially gamed? Is that acceptable for V1 given quality is explicitly unjudged?
2. Is `task_kind` immutability right, or is there a legitimate reclassification
   need that immutability blocks?
3. Under multiple reopens (multiple `task_started`), which seq boundary defines
   "during task" — and can the choice be gamed to reset the discovered count?
4. Does surfacing "new uncertainties discovered: N" risk becoming a covert metric
   (read as "good spike" or "bad spike") despite the no-verdict rule?
5. `artifact_kind`: fixed enum vs free string — which serves real research better
   without inviting a taxonomy that pretends to be meaningful?
