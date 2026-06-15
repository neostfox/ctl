# Divergence — Uncertainty Ledger V1

> Originator artifact. L0 content (free text). Records the candidate directions
> considered before convergence. Not an audit fact; not a quality claim.
> Brainstorm: `BS-UL1`. Task: `uncertainty-ledger-v1`.

## Problem

`EPISTEMIC_CONTROL.md` §1.5 names the gap: cognitive artifacts have provenance
(now shipped as Brainstorm Provenance V1), but the *substance* they reason about —
the open unknowns, the assumptions taken on faith, the things later closed by
evidence — has no first-class record. A task can be all-green, fully bound, fully
consistent, and still rest on unexamined assumptions that nobody can enumerate
after the fact (§1.4). The Uncertainty Ledger is the **record-and-disclose** layer
for those unknowns. It does not judge whether the thinking was good; it makes the
remaining uncertainty *visible and sourced* (§3).

This artifact diverges across the design axes before committing. Each axis lists
the candidate directions and the originator's lean — the lean is **not** the
decision; convergence (post-critic) decides.

## Axis A — Object model

- **A1. Single `Uncertainty` object.** One lifecycle:
  `open → resolved | accepted_as_assumption | invalidated`. (§5.1)
- **A2. Claim / Evidence / Unknown trichotomy.** Three aggregates with relations.
- **A3. Richer ontology** — claims, evidence, assumptions, risks, each typed.

Lean: **A1.** §5.1 is explicit: "a Claim closed by evidence *is* an Unknown closed
by evidence." Three heavy aggregates re-open the DESIGN.md vision>implementation
gap the doc warns against (§5 preamble). One object, one lifecycle.

## Axis B — Field set

- **B1. Minimal:** `id`, `statement`, `source`, `status`, `evidence_ref?`.
- **B2. + `impact` (4-level), `blocking`, `confidence`, relations.** (§5.1 "leave for later")
- **B3. + `source_run_id`** instead of free `source`.

Lean: **B1.** §5.1 pins the minimal field set and defers impact/blocking/confidence
to "later schema evolution, not V1." `confidence` is an explicit non-goal (§9 —
"pseudo-precise scores become another green check"). `source_run_id` (B3) presumes
a run layer that does not exist (M6+, §2); a free-text `source` is the honest
unattested placeholder for now.

## Axis C — Storage / trust level

- **C1. Canonical L2 events** (`uncertainty_recorded`, `uncertainty_disposition_recorded`)
  participating in replay, with seq, idempotency, reducer projection.
- **C2. L0 free-text file** under the brainstorm dir.
- **C3. L1 telemetry index** entries.

Lean: **C1.** The ledger's value is that disposition history is *replayable and
ordered* — when did an unknown move from open to resolved, and on what evidence.
That is L2 (§4). But heed §4's warning: an L2 envelope wrapping an L0 claim proves
*it was recorded faithfully*, not *that the statement is true*. The `source` and
`evidence_ref` remain content the control layer cannot verify.

## Axis D — Disposition event shape

- **D1. One `uncertainty_disposition_recorded`** carrying a `disposition` enum
  (`resolved` | `accepted_as_assumption` | `invalidated`) + optional `evidence_ref`
  + optional reason.
- **D2. Three separate events** — one per terminal transition.

Lean: **D1.** Smaller enum surface; one payload schema to validate. The directive
agrees: "a single disposition event can carry all three — smaller than a separate
event type per transition."

## Axis E — `resolved` integrity

- **E1. `resolved` requires `evidence_ref`** (reducer rejects without it).
- **E2. `evidence_ref` optional even on resolved.**

Lean: **E1.** §5.1: "resolved MUST carry `evidence_ref`; an unknown resolved with
no external evidence is only marginally better than open." The reducer should
reject `resolved` without `evidence_ref`. `accepted_as_assumption` must remain
*visibly unresolved by external evidence* — it is the honest label for "we chose to
proceed on faith," and must never be silently upgradable to resolved.

## Axis F — Disclosure

- **F1. Raw facts only** — counts per status + per-item list with source/evidence.
  No aggregate verdict. (§6)
- **F2. Aggregate epistemic verdict** (e.g. `PARTIALLY_VALIDATED`, a score, a light).

Lean: **F1**, hard. §6 forbids any single epistemic verdict: "give it a word and
you have built a smaller green check — either a yellow light everyone learns to
ignore, or a lever to game (mark resolved to upgrade the label)." Status output
shows `open / accepted_as_assumption / resolved / invalidated` counts and then the
items, each with its `source` and `evidence_ref`. No green/yellow/red roll-up.

## Axis G — Enforcement

- **G1. Record-only.** Never gates create/finish; never blocks on open unknowns.
- **G2. Finish-gate on open/blocking uncertainties.**

Lean: **G1** for V1. §7's control-loop-inversion warning and §9 ("no hard phase
gates in V1") both point here, as does Brainstorm Provenance V1's own precedent
(record-only, never gates). Whether a *narrow* later gating rule is justified is a
post-dogfood question — V1 must not pre-judge it.

## What V1 explicitly does NOT build (carried from §9 + directive)

Claim ontology · risk matrix · confidence/impact scores · PRD coverage · blocking
propagation · automatic stale marking · dependency graph / propagation engine ·
critic-independence attestation · requirement/design binding · active finish gating
on open uncertainty · any aggregate epistemic verdict.

## Open questions for the critic

1. Is a free-text `source` honest enough, or does shipping it risk being read as
   provenance it cannot back (the §3 "lab coat on a green check" failure)?
2. Does `accepted_as_assumption` need a structural guard so it cannot be quietly
   flipped to `resolved` without evidence, or is the reducer rule on `resolved`
   sufficient?
3. Is `invalidated` distinguishable in the record from `resolved` in a way that
   prevents "I was wrong" from masquerading as "I proved it"?
4. With record-only and no gating, what makes this *more* than a structured
   self-report — i.e., where is the externally-checkable surface in V1, if any?
