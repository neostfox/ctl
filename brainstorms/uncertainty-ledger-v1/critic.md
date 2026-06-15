# Critic — Uncertainty Ledger V1

> Independent critique of `brainstorms/uncertainty-ledger-v1/divergence.md`, grounded in `EPISTEMIC_CONTROL.md` (§§2–9) and the shipped Brainstorm Provenance V1 (`src/domain/task.rs`, `schemas/control.event-envelope.v1.schema.json`). L0 content. Not an audit fact. Produced in a separate context from the originator; independence is **unattested** (no run layer — §2/§8).

## Verdict in one line

The leans are mostly correct and well-anchored to the spec — but the artifact is *too comfortable*. It cites §3/§4/§6 as if quoting the warning inoculates the design against it. It does not. Two of the four open questions are answered by the divergence with hand-waving, and one structural gap (`accepted_as_assumption` ↔ `resolved`) is asserted-away rather than designed-against. Below, concrete positions.

## Answers to the four open questions

**Q1 — Is free-text `source` honest enough, or does it risk being read as provenance it can't back?**

Position: **Ship `source`, but only if it is structurally branded as a claim, not provenance — and rename it.** The word "source" is itself the §3 failure: it *connotes* attestation. The shipped precedent already solved this exact problem: BS-provenance records `source_run_id` but the *view* carries a sibling field `source_run_attested: false` (task.rs:210–211) so the disclosure layer can never render the claim as established. The Uncertainty ledger must do the same. A bare free-text `source` with no `*_attested: false` companion in the view is strictly *weaker* than what already shipped, and it is the literal "lab coat on a green check." So: keep the field, but (a) rename to `source_note` or `claimed_source` to kill the provenance connotation, and (b) the status view must emit it adjacent to an explicit unattested marker, never as a column titled "Source" that reads like an audit fact. Without that, `source` should NOT ship.

**Q2 — Does `accepted_as_assumption` need a structural guard against silent flip to `resolved`, or does the reducer rule on `resolved` suffice?**

Position: **The reducer rule on `resolved` is NOT sufficient. You need a structural guard, and it is one line.** The divergence's own E1 text says assumption "must never be silently upgradable to resolved" — but the proposed mechanism (reject `resolved` without `evidence_ref`) does not enforce that at all. Nothing stops a disposition event carrying `disposition=resolved` + *any* string in `evidence_ref` — including the assumption's own reasoning re-pasted as "evidence." The reducer checks **presence**, not **externality**, of evidence. So the gap the originator names is left wide open. Concretely, add **transition guards in the reducer**: (1) a terminal status is terminal — once `resolved | invalidated | accepted_as_assumption` is recorded, a *second* disposition event must be rejected unless it is an explicit, separately-typed `reopen`. This makes assumption→resolved impossible to do *silently*; it requires a visible reopen in the event log. (2) `evidence_ref` on `resolved` must point *outside* the Uncertainty object itself (a different artifact ref/hash), not be free text — otherwise §5.1's "可区分被断言关闭与被外部 oracle 关闭" distinction is decorative. This is the single most important change before convergence.

**Q3 — Is `invalidated` distinguishable from `resolved` so "I was wrong" can't masquerade as "I proved it"?**

Position: **Only if you forbid `evidence_ref` from doing double duty, which the current design does not.** As written, both `resolved` and `invalidated` flow through the same `uncertainty_disposition_recorded` event (D1). The enum value distinguishes them in the *record*, which is good and sufficient at the data layer. The real risk is at the **disclosure** layer (Q3 is really a §6 question): if the status view buckets both under a generic "closed" count, or renders `invalidated` next to an `evidence_ref` the same way `resolved` is, a reader conflates "the unknown was disproven / became moot" with "the claim was validated." Mandate: the view keeps four *distinct* status buckets (the divergence's F1 already does this — defend it), and `invalidated` should be displayed with a *reason*, not an `evidence_ref`-shaped field, because "I was wrong" has no oracle. The data model is fine; the burden is on never collapsing the four statuses into "open vs. done."

**Q4 — With record-only and no gating, what makes this more than a structured self-report?**

Position: **Be honest — in V1, almost nothing. And that is acceptable, but only if the system says so out loud.** This is the artifact's central tension and it under-answers it. §2/§3 are explicit: there is no independent orchestrator, no run layer (`source_run_id` presumes M6+), no external oracle. So the *only* externally-checkable surfaces V1 actually delivers are:
1. **Tamper-evident ordering of disposition history** — L2 seq + idempotency means "this unknown moved open→resolved at seq N citing artifact-hash H" is a faithfully-recorded, replayable fact (envelope integrity, §4). That is real and it is more than a flat L0 file (C2/C3 can't give ordered replay).
2. **Hash-bound `evidence_ref`** — *if* (per Q2) evidence_ref is a path+hash like `ArtifactRef`, then staleness is lazily derivable (`current_digest != recorded_digest`, §5.2/§9), so a reader can mechanically check "the evidence this resolution cited no longer exists / changed." That is an external check the control layer *can* perform.

Everything else — whether the statement is true, whether the source is real, whether resolution was warranted — is unverifiable self-report. **That is acceptable for V1** because the spec's whole thesis (§2 note: 第五条…无法被彻底修复) is that this layer is a *long-term boundary*, not a bug to fix. Recording-with-faithful-ordering is the honest floor. But the design must surface this: the status view needs a standing disclosure equivalent to BS-provenance's pinned `unattested` — e.g. every resolved item carries `evidence_attested: false` — so the structured self-report is never mistaken for verification. The divergence gestures at this in C1's prose but does not put it in the data model. **Make it a field.**

## The single biggest weakness

**`evidence_ref` is specified as a string, not a hash-bound `BoundRef`/`ArtifactRef` — and that one choice quietly guts three of the design's claims at once.** The whole value proposition of E1 (`resolved MUST carry evidence_ref`) rests on evidence_ref meaning something the control layer can re-check. If it is free text, then: (a) Q2's anti-upgrade guard is unenforceable (any string passes), (b) Q4's "externally-checkable surface" collapses to zero, and (c) §5.1's "区分被断言关闭与被外部 oracle 关闭" becomes impossible because there is no structural difference between "evidence_ref: 'I checked, it's fine'" and "evidence_ref: tests/foo.rs@<hash>". The shipped `ArtifactRef { path, hash }` (task.rs:153–157) is *exactly* the primitive this needs and it already exists. Reusing it costs almost nothing and converts evidence_ref from decoration into the one genuinely load-bearing, machine-checkable field in the feature. The divergence treats this as a minor field-shape detail (B1 lists `evidence_ref?` as a bare optional); it is in fact the hinge the entire feature swings on.

## Where the design risks the exact failure the spec warns against

- **§4 envelope-integrity ≠ content-trust:** The C1 lean is right to choose L2, and the prose acknowledges the warning. The *risk* is in the unwritten disclosure layer. BS-provenance enforced this with a reducer that **pins** `trust_level=content_l0` and rejects anything higher (task.rs:394–397, `check_trust_level`). The Uncertainty divergence does **not** propose an equivalent pinned field. Without it, an L2 `uncertainty_recorded` event will be read as "the control plane vouches for this unknown." Mandatory: pin a content trust level on every uncertainty event exactly as the precedent does.
- **§6 no aggregate verdict:** F1 is correctly hard-line. The subtle trap is *counts themselves becoming a verdict*: "12 resolved / 1 open" reads as a 92% green-ish score even with no roll-up word. §6's deeper point ("标 resolved 升级标签") is that the *act of resolving* is the gameable lever. The counts are fine to show, but the view must not sort/color/badge them in a way that manufactures the verdict §6 forbids. Defend F1, but add: no ratios, no percentages, no ordering that implies progress.
- **§3 "lab coat on a green check":** see Q1. Recording the uncertainty map without recording *its own* provenance is precisely the §3 推论 failure. In V1 with no run layer, the only honest provenance is `recorded_by` (the actor) + the unattested marker. Make sure both ship.

## Concrete changes / additions before convergence

1. **CHANGE `evidence_ref: string` → `evidence_ref: ArtifactRef { path, hash }`** (reuse the shipped type). This is the biggest-weakness fix; everything else is downstream of it.
2. **ADD a reducer transition guard:** terminal status is terminal; a second disposition on an already-disposed uncertainty is rejected unless via an explicit `uncertainty_reopened` event. This is the structural anti-silent-upgrade guard Q2 demands, and it makes assumption→resolved a *visible* log event rather than an in-place flip.
3. **ADD a pinned content trust level + `evidence_attested: false`** on the view, mirroring `BRAINSTORM_TRUST_LEVEL` / `source_run_attested`. This is what actually keeps the L2 ledger from being read as content-trust (§4), and it is the precedent the codebase already set one feature ago.
4. **RENAME `source` → `source_note`/`claimed_source`** and render it adjacent to an unattested marker (Q1).

## What the originator got RIGHT — defend against scope creep

- **F1 (fact-only, no aggregate verdict) is exactly correct and is the spiritual core of §6.** Every future iteration will feel pressure to add "just a little summary light" or an `epistemic: PARTIALLY_VALIDATED` so dashboards have something to show. That pressure must be refused. The single object + four explicit status buckets with no roll-up is the right shape and matches the shipped BS-provenance philosophy (a fact-only `*ProvenanceView` carrying "no pass/fail verdict," task.rs:195–215). Defend F1 verbatim.
- Secondary defends: **A1** (single object — §5.1 is unambiguous and A2/A3 re-open the vision>implementation gap §5 warns against) and **G1** (record-only, no gating — §7 control-loop-inversion + §9 + BS-provenance precedent). Both are correct; resist the inevitable "but if we have the data, why not gate finish?" at convergence — §7 already answered that.

## Revised uncertainty set

Open unknowns I believe remain after this critique:

1. **Evidence externality is undefined.** Even with `ArtifactRef`, nothing proves the cited artifact is *external* to the reasoning that produced the resolution (e.g. citing the brainstorm artifact itself as "evidence"). V1 can detect staleness but not self-citation. Open: is a same-task / same-brainstorm hash an acceptable `evidence_ref`, and if not, who enforces that?
2. **Reopen semantics.** If change #2 (terminal-is-terminal + explicit reopen) is adopted, the reopen event shape, who may emit it, and how the status view shows churn history are unspecified.
3. **Uncertainty ↔ task lifecycle coupling.** The divergence never says whether uncertainties are bound to a task (like `brainstorm_ref`, one-per-task) or free-floating per brainstorm. The reducer's id-collision rules (cf. task.rs:1099–1105) depend on this and it is unresolved.
4. **The §3-recursion (provenance of the uncertainty map itself) is only partially closed** by `recorded_by` + unattested marker. Whether that is "enough disclosure" or merely the least-dishonest available option in a no-run-layer world is itself an open epistemic question — and, fittingly, one this ledger cannot resolve, only record.
