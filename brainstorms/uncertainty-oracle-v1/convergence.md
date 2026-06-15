# Convergence — Task Proposal: uncertainty-oracle-v1 (BS-UO1)

> L0 content. The originator's converged proposal after the independent critic pass.
> This is the authoritative scope for the task; it does not assert correctness.

## Objective

Promote **Evidence** to a first-class, oracle-typed ledger object, so the control layer
can disclose *what kind of oracle* closed each unknown — without ever scoring, gating,
or rendering an epistemic verdict. Builds strictly on the existing Uncertainty Ledger
(`open → resolved | accepted_as_assumption | invalidated`); does not rebuild it.

OMP discovers/describes/explains unknowns; ctl records, source-binds, transitions,
constrains by evidence, and discloses facts.

## Domain model (minimal)

One new object + one extended one:

```
Evidence {
  evidence_id     // stable, unique within the task (e.g. E-001)
  oracle_kind     // fixed enum (below)
  source_ref      // free-text locator (command, test name, URL); UNATTESTED
  artifact_ref    // ArtifactRef (path + ctl-computed hash); file-backed, REQUIRED
  recorded_by     // = envelope actor at record time (one principal, not a 2nd field)
}
```

`oracle_kind` (fixed enum, no free string, no `other`):
`deterministic | test | runtime | human | model | external_authority`

Semantic pins (enforced where possible, disclosed always):
- `model` is **advisory** — never rendered as fact/truth; carries an explicit marker.
- `human` ≠ authenticated principal; `source_ref`/`recorded_by` are unattested.
- run authenticity / model-run / critic independence: `unattested` or `unavailable`.
- L2 envelope integrity of an evidence event never raises its content trust (stays L0).

`Uncertainty` is extended only so a `resolved` can name a recorded evidence:
- new optional `evidence_ref` (= an `evidence_id`) set when resolved-via-recorded-evidence.
- legacy inline `evidence_ref: ArtifactRef` (path+hash on resolve) is retained for replay.

## Canonical events (evaluate-then-implement, per EPISTEMIC_CONTROL §8)

- **NEW** `evidence_recorded` — `{ evidence_id, oracle_kind, source_ref?, artifact_path,
  artifact_hash, trust_level: content_l0 }`. `recorded_by` is derived from the envelope
  actor (not in payload). One event type for all oracle kinds (no per-kind event).
- **EXTEND** `uncertainty_disposition_recorded` — add optional `evidence_ref`
  (an evidence_id). Unchanged for accepted_as_assumption / invalidated.
- `uncertainty_recorded` — unchanged.

No new disposition event types; disposition stays one event keyed by `disposition`.

## Reducer invariants (the real contract)

1. `evidence_recorded`: `evidence_id` unique within task; `oracle_kind` ∈ enum;
   `artifact_path`+`artifact_hash` required; `trust_level` must be `content_l0`.
2. `resolved` may carry **either** `evidence_ref` (must reference an already-recorded
   evidence in this task) **or** legacy inline `evidence_path`+`evidence_hash`,
   **never both** (reject "both"); **never neither** (resolved still requires evidence).
3. `accepted_as_assumption` / `invalidated`: must NOT carry evidence_ref or inline
   evidence (unchanged); invalidated still requires a reason.
4. terminal-is-terminal (unchanged): a disposed uncertainty cannot be re-disposed —
   an assumption can never be silently upgraded to resolved.
5. unknown `evidence_ref` (no matching `evidence_recorded`) → clear rejection,
   mirroring the existing "unknown uncertainty" rejection.
6. Legacy/absent streams replay byte-identically (new fields optional; default empty
   `evidences` vec).

## Disclosure (fact-only — §五)

Add an `ORACLE SOURCES` block and surface uncertainties in BOTH surfaces:
- `ctl uncertainty status` (human + --json): extend with ORACLE SOURCES + per-item
  `evidence_ref` / `oracle_kind`.
- `ctl task status` (human + --json): NEW — render the uncertainty ledger + ORACLE
  SOURCES (today it shows neither), behind the same
  `(content: unverified; evidence: unattested)` banner.

ORACLE SOURCES breakdown (raw counts, e.g.):
```
ORACLE SOURCES
  deterministic/test: 2
  runtime: 0
  human decisions: 1
  model advisory: 2
  external authority: 0
```
Per uncertainty: `id, statement, status, source, evidence_ref, oracle_kind`. A resolved
item backed by a `model` oracle renders `oracle: model — ADVISORY (not external proof)`.

**Forbidden output (hard):** `EPISTEMIC: PASS`, `Spec confidence: N%`, any evidence-
strength score, any red/yellow/green or aggregate epistemic verdict. Epistemic dimension
discloses texture only.

## Explicit non-goals (V1)

Oracle Registry; research quality scoring; requirement coverage; PRD; active invalidation
propagation; generic dependency graph; Claim/Evidence/Unknown tri-ontology; authenticated
principal; independent orchestrator; L3 hash chain; requirement/design binding;
`tree_hash` evidence (dropped per critic C6); open-uncertainty finish hard-gate
(record-and-disclose only — never blocks create/finish).

## Boundary / scope

- **write_allow:** `src`, `brainstorms/uncertainty-oracle-v1`.
- **Protected schema** `schemas/control.event-envelope.v1.schema.json` is edited via the
  M-f reviewed exception (`ctl apply` → granted after a ctl-review mode-A pass), exactly
  as bs-prov / uncertainty-ledger / research-spike did. Not added to write_allow.
- **gates:** cargo_fmt_check, cargo_check, cargo_test, cargo_clippy — all bound to the
  committed tree_hash + policy_hash.
- Respect scope, protected path, gate, tree_hash and policy_hash interlocks throughout.

## Files expected to change

- `schemas/control.event-envelope.v1.schema.json` (via ctl apply): enum + `evidence_recorded`
  block + optional `evidence_ref` on disposition.
- `src/domain/task.rs`: `Evidence` struct, `OracleKind` enum, `TaskState.evidences`,
  reducer arm for `evidence_recorded`, extended `uncertainty_disposition_recorded` arm,
  view structs (oracle breakdown + evidence_ref/oracle_kind on item views).
- `src/application/mod.rs`: `record_evidence`, extend `record_uncertainty_disposition`
  (evidence_ref path), oracle-source aggregation in the ledger view, plumb into status.
- `src/cli/mod.rs`: `ctl uncertainty evidence` (record) + extend `dispose` (--evidence-ref),
  ORACLE SOURCES rendering, uncertainty block in `ctl task status` (human + json).

## Test plan (§七, ≥12)

1. record open uncertainty. 2. resolved with valid evidence_ref → ok. 3. resolved without
any evidence → rejected. 4. accepted_as_assumption needs no evidence, shows unproven.
5. invalidated records reason/source. 6. disposition on nonexistent uncertainty → rejected.
7. evidence_ref → nonexistent evidence → rejected. 8. model oracle shown advisory/unattested.
9. legacy stream with no uncertainty/evidence data replays unchanged. 10. neither human nor
JSON status emits an epistemic verdict. 11. research/spike artifact usable as evidence
source_ref / artifact. 12. schema+reducer+CLI consistently reject illegal states. Plus:
13. resolved carrying BOTH evidence_ref and inline evidence → rejected (critic C1).
14. evidence_recorded duplicate evidence_id → rejected.

## Provenance note

This brainstorm (BS-UO1) is L0 content. The independent critic was a separate read-only
agent (same model family, no trusted orchestrator) → independence is `unattested`. The
canonical brainstorm events prove only that these artifacts were referenced, not that the
thinking was correct or the critic truly independent.
