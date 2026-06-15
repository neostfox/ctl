# Convergence — Research/Spike V1 (BS-RS1)

> Convergence proposal / Task Proposal. L0 content. Resolves the originator
> (`divergence.md`) ↔ critic (`critic.md`) delta into a buildable V1.
> Task: `research-spike-v1`.

## Originator ↔ critic delta (what changed because of the challenge)

| # | Critic finding | Disposition | Rationale |
|---|----------------|-------------|-----------|
| Factual | `task_started` fires **once** (Ready→InProgress, guarded); reopen emits `task_reopened`, not a second `task_started`. Axis C/Q3's "multiple task_started" premise was wrong. | **ACCEPT** | Verified against `task.rs:624`/`642`. Axis C rewritten: anchor "during task" to the single `task_started` seq; reopen does **not** advance it. |
| C1 | Drop "new uncertainties discovered: N" as a scalar — it is rankable (covert §6 metric) **and** manufacturable by recording uncertainties in Planning before `start` (no phase guard on `uncertainty_recorded`). | **ACCEPT** (user-confirmed) | No count line. Discovered uncertainties appear only as **per-item tags** (`recorded after start`) in the fact-only list — no rankable subtotal. Neutralizes covert-metric + sequencing-manufacturability in one move. |
| C2 | Bind artifact **freshness** in the disclosure, reusing the ledger's `CURRENT/STALE/ABSENT` derivation. | **ACCEPT** | A recorded artifact whose file was deleted/mutated must read STALE/ABSENT, else completion can point at a vanished `findings.md` and still read "1 artifact". Reuses the shipped freshness primitive — cheap, in-scope. |
| Q5 | Drop `experiment` from `artifact_kind`; ship `findings\|recommendation\|design_draft`, no `other`. | **REJECT (keep all four)** | The directive enumerates `findings\|experiment\|recommendation\|design_draft` and gives `research/<id>/experiment-results.json` as an example artifact — `experiment` labels a real document type, not a runner. Kept per directive; critic's "no runner" objection noted. Fixed enum (no `other`) per critic — that part accepted. |
| Bar | Reframe the completion bar as a **non-degeneracy minimum**, not a quality screen; note "≥1 uncertainty outcome" collapses to "≥1 `uncertainty_recorded`" (dispositions require a prior record). | **ACCEPT** | §6/§9 forbid judging quality; the two checks only stop a research task completing with zero evidentiary/epistemic footprint (looking identical to an implementation task that produced nothing). Keep the checks; demote the rhetoric. |
| Immutability | `task_kind` immutable after create. | **ACCEPT** | Each mutable path opens a named integrity hole (research→impl dodges the artifact rule; impl→research launders failed gates). Reclassification is served by cancel + new task. |
| U-R5 | Confirm the two research checks run over **all** events for the task (cumulative), not "since last reopen". | **ACCEPT** | Derived from full stream in `finish_task`; consistent with how reopen re-gating already works. |
| U-R3 | The reducer's lack of a phase guard on `uncertainty_recorded` is a latent property of shipped Uncertainty Ledger V1; record it, don't fix it here. | **ACCEPT** | Will record as a new uncertainty against `uncertainty-ledger-v1` (out of scope to change). |

Defended against scope creep (critic agreed): **one event, no `research_completed`** (a summary event would duplicate derivable state and read as "sufficiently researched" — the §7 compression); **reuse `ArtifactRef` + pin `content_l0` / `attestation: unavailable`** (no new trust model).

## Converged V1 spec

### `task_kind`

`enum TaskKind { Implementation, Research }`, field on `TaskState`, `#[serde(default)]
→ Implementation` (legacy & absent streams replay unchanged). Set at `task_created`
from an optional payload field; **immutable** (never changed by `task_revised`).
CLI: `ctl task create --kind <implementation|research>` (default implementation).

### Canonical event (exactly one new)

`research_artifact_recorded` — payload:
```
artifact_path     (normalized repo-relative; ctl computes the hash)
artifact_hash     (SHA-256, ctl-computed)
artifact_kind     findings | experiment | recommendation | design_draft  (fixed enum, no `other`)
source_run_id?    optional, UNATTESTED claim
trust_level       pinned content_l0 (reducer rejects anything higher)
```
Reduced into `state.research_artifacts: Vec<ResearchArtifact>`. No `research_completed`.

### Completion (research tasks only — after the normal integrity checks)

A research task is **not** exempt from execution integrity: committed tree ==
gate/audit tree, policy match, required gates pass, fresh completion audit — all
still apply. In addition, in `finish_task`, when `task_kind == Research`:
1. **≥1 `research_artifact_recorded`** (at least one tracked output artifact), and
2. **≥1 `uncertainty_recorded`** (at least one uncertainty outcome; dispositions
   require a prior record, so this is the real floor).

Checks are over the full event stream (cumulative). **Never** requires the open
count to decrease — a spike that opens four hidden risks is a success.

### Disclosure — `RESEARCH OUTPUT` (shown for `task_kind=research`; raw facts only)

```
RESEARCH OUTPUT
  artifacts produced: 2
  uncertainties opened: 3
  resolved with evidence: 1
  accepted as assumptions: 1
  invalidated: 0

  ARTIFACTS
    findings        research/<id>/findings.md @ <hash>   freshness: CURRENT  (attestation: unavailable)
    recommendation  design/<id>/rec.md @ <hash>          freshness: CURRENT  (attestation: unavailable)
  UNCERTAINTIES
    U-1  RESOLVED   [recorded after start]  evidence: ... CURRENT
    U-2  OPEN       [recorded after start]
    U-3  OPEN       [pre-start]
```
No `new discovered` count line. The "recorded after start" tag is a per-item fact
(derived: the uncertainty's `uncertainty_recorded` seq > the single `task_started`
seq), not a rankable subtotal. No verdict, score, percentage, or ratio. `source_run`
on artifacts, if present, is rendered as an unattested claim. Trust `content_l0`.

### Out of scope (carried from directive + critic)

Methodology engine · claim ontology · experiment runner · web research · requirement
coverage · **fewer-unknowns success metric** · whole-brainstorm skip · U-2 evidence
externality · principal auth · any aggregate research verdict/score · `research_completed`
event · `artifact_kind` `other`/free string · phase guard on `uncertainty_recorded`.

### Recorded as follow-up (not built here)

- **New uncertainty against `uncertainty-ledger-v1`**: `uncertainty_recorded` has no
  phase guard (U-R3) — uncertainties can be recorded in any phase. Latent property;
  record-and-disclose, decide later.

---

## Task Proposal

```
Task Proposal: research-spike-v1   (task_kind: implementation — it builds the feature)
  Objective:  Add a minimal research task kind whose completion discloses tracked research
              artifacts and uncertainty outcomes without treating uncertainty reduction as
              a success metric.
  Read:       src, schemas, fixtures, Cargo.toml, EPISTEMIC_CONTROL.md
  Write:      src                                            (domain/application/cli + tests)
              schemas/control.event-envelope.v1.schema.json  (NEEDS step-up approval — protected;
                                                               new event + task_created task_kind)
              brainstorms/research-spike-v1                   (these artifacts)
  Deny:       (.git, .ctl, Cargo.toml, schemas protected by default)
  Gates:      cargo_fmt_check, cargo_check, cargo_test, cargo_clippy
  Risks:      - schema touches a protected path → approval (precedent: bs-prov-v1, uncertainty-ledger-v1)
              - finish interlock change must not affect implementation tasks (kind-gated)
              - disclosure must never emit a discovered scalar or any verdict (§6)
              - reuse the ledger's freshness primitive; do not fork a second trust model (§9)
  Specs:      EPISTEMIC_CONTROL.md §3/§6/§7/§9; shipped Uncertainty Ledger V1 + brainstorm-provenance
  Provenance: BS-RS1 — divergence + independent (unattested) critic + this convergence
```
