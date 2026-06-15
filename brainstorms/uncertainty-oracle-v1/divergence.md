# Divergence — uncertainty-oracle-v1 (BS-UO1)

> L0 content. This is an originator's candidate-direction sketch. It is unverified,
> may contain errors, and has not been independently challenged at the time of writing.
> Recording it as brainstorm provenance never raises its trust level.

## Problem (one line)

The uncertainty ledger already records `open → resolved | accepted_as_assumption |
invalidated` and already forces `resolved` to carry hash-bound evidence. What it
**cannot** do is say *what kind of oracle* closed an unknown. A model's advisory
guess and a deterministic test are recorded identically (an inline `(evidence_path,
evidence_hash)`). EPISTEMIC_CONTROL §5.1 explicitly demands the distinction between
"被断言关闭" and "被外部 oracle 关闭". uncertainty-oracle-v1 is the minimal step that
makes evidence a first-class, oracle-typed object — record-and-disclose only.

## What already exists (do NOT rebuild)

- `Uncertainty { id, statement, source, status, evidence_ref: Option<ArtifactRef>, reason }`
- events `uncertainty_recorded`, `uncertainty_disposition_recorded`
- reducer invariants: terminal-is-terminal; resolved requires evidence; assumption/
  invalidated must NOT carry evidence; invalidated requires reason
- `ArtifactRef` (path+ctl-hash) + lazy freshness CURRENT/STALE/ABSENT
- `ctl uncertainty status` (human + --json) via `uncertainty_ledger_view`
- `ctl task status` shows brainstorm provenance + (research kind) research output —
  but does NOT show the uncertainty ledger today.

## The hard constraint discovered up front

There are **committed** `uncertainty_disposition_recorded` events with
`disposition: resolved` carrying inline `evidence_path` + `evidence_hash` and NO
`evidence_ref` (uncertainty-ledger-v1 U-1, U-3). Any "resolved must reference a
recorded evidence" invariant therefore CANNOT be a blanket schema/reducer rejection
of resolved-without-evidence_ref — that would break legacy replay (an explicit V1
non-negotiable). The new invariant must apply to the *new evidence_ref shape*, not
retroactively to the legacy inline shape.

## Candidate directions

### D1. Evidence as a first-class recorded object (`evidence_recorded` event)

Add a third event `evidence_recorded { evidence_id, oracle_kind, source_ref,
artifact_path?+artifact_hash? | tree_hash?, recorded_by }`. Evidence lives in
`TaskState.evidences: Vec<Evidence>`. `resolved` gains an `evidence_ref` field
(= an evidence_id) that the reducer requires to point at an already-recorded
evidence in the same task. Legacy inline evidence_path/hash still accepted on
resolved for replay; new CLI always goes through record-evidence-then-reference.

- Pros: matches EPISTEMIC_CONTROL §8 minimal vocabulary exactly; oracle_kind lives
  on the evidence where it belongs; one evidence can back multiple uncertainties;
  recorded_by captured; clean separation "evidence is a thing, disposition references it".
- Cons: two CLI steps to resolve (record evidence, then dispose). New event + schema
  changes (M-f apply). Must keep dual-path reducer (evidence_ref OR legacy inline).

### D2. Inline oracle_kind on the disposition (no new event)

Just add `oracle_kind` (+ `source_ref`) to `uncertainty_disposition_recorded`.
Evidence stays inline on the resolve.

- Pros: smallest change; one CLI step; no new event.
- Cons: no first-class Evidence (can't reuse, can't record evidence separately from a
  resolution, no evidence_id, no recorded_by-as-evidence-fact); diverges from §8's
  named `evidence_recorded`; the task spec §二/§四 explicitly want Evidence with an
  evidence_id and an `evidence_recorded` event. Rejected as under-reaching the ask.

### D3. Full Oracle Registry (oracle sources as their own tracked entities)

Model oracle *sources* (a registry of named oracles with their own provenance).

- Pros: richest.
- Cons: the task spec explicitly says "不要建立庞大的 Oracle Registry"; oracle_kind is a
  fixed enum, not a registry. Over-reach. Rejected.

### D4. oracle_kind taxonomy

Fixed enum (no free string, mirroring ResearchArtifactKind / no `other`):
`deterministic | test | runtime | human | model | external_authority`.
Semantic pins to enforce/disclose:
- `model` evidence is ALWAYS advisory — never rendered as fact/truth.
- `human` ≠ authenticated principal (envelope actor is still just a string).
- run authenticity / model-run / critic independence: unattested or unavailable.
- L2 envelope integrity of an evidence event never raises the content trust of its claim.

### D5. Where to surface disclosure

(a) Extend `ctl uncertainty status` with an ORACLE SOURCES block + per-item
    evidence_ref/oracle_kind. (b) ALSO add an UNCERTAINTIES + ORACLE SOURCES block to
    `ctl task status` human and `--json` (today it shows neither). Both fact-only:
    raw counts per oracle_kind, model flagged advisory, NO verdict / NO score / NO
    red-yellow-green / NO "resolved%".

### D6. evidence binding: artifact_ref vs tree_hash

Evidence should bind to EITHER a file (`artifact_path`+ctl-hash, reuse `hash_evidence`)
OR a `tree_hash` (for runtime/deterministic evidence not captured as a single file).
At least one required; both allowed? Keep minimal: exactly one of {artifact, tree_hash},
plus always `source_ref` (free-text locator, unattested) + `oracle_kind` + `recorded_by`.

## Leaning

D1 + D4 + D5 + D6, strictly record-and-disclose. New event `evidence_recorded`;
`evidence_ref` on resolve referencing it; legacy inline path preserved for replay;
oracle_kind fixed enum with `model`=advisory pin; disclosure added to BOTH status
surfaces, fact-only, no aggregate verdict.

## Open questions for the critic

1. Should the NEW resolve path forbid inline evidence entirely (force evidence_ref),
   or accept both? (Leaning: new CLI emits evidence_ref only; reducer still accepts
   legacy inline for replay; reject events that carry BOTH for the same resolve.)
2. Is `tree_hash` evidence worth it in V1, or does it invite a fake "runtime" oracle
   with no real artifact? (It's unattested either way — disclose, don't trust.)
3. Does adding UNCERTAINTIES to `ctl task status` risk re-creating a green check by
   proximity to gate PASS lines? (Mitigation: explicit "content: unverified" banner,
   no totals/score, model flagged advisory.)
4. Must `evidence_recorded` be allowed before `task_started` / in any phase? (Record-
   only; no phase guard — consistent with uncertainty U-4's disclosed latent property.)
5. recorded_by — is it the envelope actor (already present) or a separate payload
   field? Separate field risks contradicting the envelope actor. (Leaning: derive from
   envelope actor; do NOT add a forgeable second principal field. Re-examine.)
