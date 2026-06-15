# Convergence — Uncertainty Ledger V1

> Convergence proposal / Task Proposal. L0 content. Resolves the
> originator (`divergence.md`) ↔ critic (`critic.md`) delta into a buildable V1.
> Brainstorm: `BS-UL1`. Task: `uncertainty-ledger-v1`.

## The originator ↔ critic delta (what changed because of the challenge)

| # | Critic ask | Disposition | Rationale |
|---|------------|-------------|-----------|
| 1 | `evidence_ref: string` → `ArtifactRef { path, hash }` | **ACCEPT** | Reuses the shipped `ArtifactRef` (task.rs:153). Converts the one load-bearing field from decoration into a machine-checkable, staleness-derivable ref (§5.2/§9 lazy `current_digest != recorded_digest`). This is the only externally-checkable surface V1 has (critic Q4.2); without it E1 is theater. |
| 2 | Structural guard against silent `assumption → resolved` | **ACCEPT, minimal form** | Reducer enforces **terminal-is-terminal**: once an uncertainty is `resolved \| accepted_as_assumption \| invalidated`, a second disposition is **rejected**. This kills silent upgrade *without* adding a third event — V1 stays at two canonical events (directive constraint). A visible `uncertainty_reopened` is **deferred to V2** (see Deferred). |
| 3 | Pin content trust level + `evidence_attested: false` in the view | **ACCEPT** | Mirrors the shipped `BRAINSTORM_TRUST_LEVEL` pin + `source_run_attested:false` (task.rs:210, 394). Stops an L2 `uncertainty_recorded` event from being misread as the control plane vouching for the unknown (§4). Cheap, and the precedent is one feature old. |
| 4 | Rename `source` → `source_note`/`claimed_source` | **PARTIAL** | Keep the field name `source` (directive names it `source`), but adopt the disclosure guard: the status view renders it adjacent to an explicit unattested marker and **never** as a column that reads like attested provenance. Rename is a naming call left to the user (see Decisions for the user). |

Defended against the critic / against scope creep (critic agreed these are right): **F1** fact-only, no aggregate verdict / no ratios / no progress-implying ordering; **A1** single object; **G1** record-only, never gates create/finish.

## Converged V1 spec

### Object — single `Uncertainty`

```
id            stable identifier, unique within its task
statement     the unknown, in plain text (L0 content)
source        free-text note on where it came from (UNATTESTED — see disclosure)
status        open | resolved | accepted_as_assumption | invalidated
evidence_ref  ArtifactRef { path, hash } — REQUIRED iff status == resolved
```

Lifecycle (terminal-is-terminal in V1):

```
open ──▶ resolved              (requires evidence_ref: ArtifactRef)
     ├─▶ accepted_as_assumption (stays visibly unresolved by external evidence)
     └─▶ invalidated            (carries a reason, NOT an evidence_ref)
(no transition out of a terminal status in V1)
```

### Canonical events (exactly two — L2, participate in replay)

- `uncertainty_recorded` — `{ uncertainty_id, statement, source, trust_level (pinned) }`
- `uncertainty_disposition_recorded` — `{ uncertainty_id, disposition, evidence_ref?, reason? }`
  - one event carries all three terminal dispositions via the `disposition` enum
  - reducer rejects: `resolved` without `evidence_ref`; `invalidated`/`assumption` *with* an `evidence_ref` (use `reason`); any disposition on an already-disposed id (terminal-is-terminal); unknown `uncertainty_id`.
  - `trust_level` pinned to the content-L0 constant exactly as BS-provenance pins it; any higher claim is rejected by the reducer.

### Status output (fact-only — §6)

```
UNCERTAINTIES  (content: unverified; evidence: unattested)
  open: 3
  accepted as assumptions: 1
  resolved with evidence: 2
  invalidated: 1

  <id>  open                 source: "<note>" (unattested)
  <id>  resolved             evidence: <path>@<hash> [STALE?]  (unattested)
  <id>  accepted_as_assumption  source: "<note>" (unattested)
  <id>  invalidated          reason: "<why>"
```

No aggregate verdict, no green/yellow/red, no percentage, no ratio, no ordering that implies progress. `resolved` items show evidence staleness (`current` vs `STALE`) derived by re-hashing, like BS-provenance.

### Explicitly out of scope (carried from §9 + directive, reaffirmed)

Claim ontology · risk matrix · confidence/impact scores · epistemic PASS/FAIL or any single verdict · PRD coverage · blocking propagation · automatic stale marking (staleness is *derived on read*, never written) · dependency graph / propagation engine · critic-independence attestation · requirement/design binding · active finish gating on open uncertainty.

## Decisions left to the user (genuinely not repo-answerable)

1. **`evidence_ref` shape.** Directive said `evidence_ref` (unspecified shape); convergence proposes `ArtifactRef { path, hash }` so it is machine-checkable. Accept the hash-bound shape, or keep a plain string? *(Recommend: ArtifactRef — it is the only thing that makes V1 more than a self-report.)*
2. **Field name `source`.** Keep `source`, or rename to `source_note`/`claimed_source` to kill the provenance connotation (critic Q1)? *(Recommend: keep `source`, rely on the unattested marker in the view.)*

## Deferred to V2 (recorded, not built)

- `uncertainty_reopened` event + reopen semantics (who may reopen, churn display).
- Evidence **externality** check — V1 detects staleness but cannot tell evidence-external-to-the-reasoning from self-citation (critic revised-uncertainty #1).
- Whether any narrow finish-gating rule is ever justified (decide only after dogfood).

---

## Task Proposal

```
Task Proposal: uncertainty-ledger-v1
  Objective:  Add Uncertainty Ledger V1 — a single record-and-disclose Uncertainty
              object (id, statement, source, status, evidence_ref) with two canonical
              events (uncertainty_recorded, uncertainty_disposition_recorded) and
              fact-only status output; reducer enforces resolved-needs-evidence and
              terminal-is-terminal; no scores, no verdict, no gating, no propagation.
  Read:       src, schemas, fixtures, Cargo.toml, EPISTEMIC_CONTROL.md
  Write:      src                              (domain/application/cli + tests)
              schemas/control.event-envelope.v1.schema.json   (NEEDS step-up approval,
                                               as bs-prov-v1 did — protected path)
              brainstorms/uncertainty-ledger-v1 (these artifacts)
  Deny:       (defaults; .git, .ctl, Cargo.toml, schemas remain protected)
  Gates:      cargo_fmt_check, cargo_check, cargo_test, cargo_clippy
  Risks:      - schema edit touches a protected path → requires approval (precedent: bs-prov-v1 seq 4-5)
              - reducer transition guards must have unit tests (terminal-is-terminal,
                resolved-needs-evidence, reject-evidence-on-invalidated)
              - disclosure layer must never collapse 4 statuses into "open vs done"
                or emit a roll-up — the §6 failure mode
  Specs:      EPISTEMIC_CONTROL.md §5.1 (ontology), §6 (disclosure), §8 (V1 events),
              §9 (non-goals); .ctl/spec/backend/* (layer guidelines)
  Provenance: BS-UL1 — divergence + independent (unattested) critic + this convergence
```
