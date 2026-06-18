# Adapter Same-Task E2E Parity Report

**Task**: `adapter-same-task-e2e-v1`
**Date**: 2026-06-18
**Result**: **PARITY** — the `omp` and `opencode` executor adapters drive the
identical ctl task lifecycle with identical governance behavior. The *only*
divergence is the evidence `source` tag (and the per-run ids/timestamps), which
is the one intended difference between the two adapters.

Harness: [`scripts/adapter_parity_e2e.sh`](./scripts/adapter_parity_e2e.sh)
(deterministic, model-free). Reproduce with `bash scripts/adapter_parity_e2e.sh`.

---

## What was compared

A single small, deterministic fixture change — bump `src/lib.rs` from
`fn answer() -> i32 { 41 }` to `{ 42 }` in a minimal throwaway cargo+git crate —
was carried through the **same** governed flow once per adapter, in isolated
temp repos. No model is invoked anywhere: the "agent output" is a pre-written
`agent-output.json` handed to `ctl run ingest --result`.

Two governed tasks per adapter:

- **HAPPY** — full lifecycle to archive (lifecycle, evidence source, gate
  binding, audit, finish/archive, adapter doctor before/after).
- **DENY** — an isolated write-scope probe (a rejected out-of-scope ingest plus
  a live `ctl hook gate`). It is a separate task because the completion
  interlock (STATE-012) deliberately blocks `finish` while an `evidence_rejected`
  is unresolved — so a deny and a completion cannot share one task.

---

## Dimension-by-dimension result

| Dimension | omp | opencode | Verdict |
|---|---|---|---|
| **Lifecycle** (event sequence) | 15 events | 15 events | **identical** |
| **Write-scope enforcement** | SCOPE-001 reject + `hook gate` deny | SCOPE-001 reject + `hook gate` deny | **identical** |
| **Evidence source** | `source="omp"` | `source="opencode"` | **differs — by design** |
| **Gate binding** | gate `tree_hash` = `HEAD^{tree}` | gate `tree_hash` = `HEAD^{tree}` | **identical (bound)** |
| **Audit** | reviewer (`ctl-review`) pass, recorded | reviewer (`ctl-review`) pass, recorded | **identical** |
| **Finish / archive** | `finish` + `archive` exit 0 | `finish` + `archive` exit 0 | **identical** |
| **Adapter doctor before/after** | byte-identical before==after | byte-identical before==after | **identical & stable** |

### 1. Lifecycle — identical 15-event ledger (HAPPY)

Both adapters produce the same ordered event stream:

```
task_created → task_marked_ready → task_started
  → lease_created → workspace_created → run_started
  → run_completed → lease_revoked → workspace_cleaned → evidence_accepted
  → task_submitted_for_review → gate_checked → evidence_accepted (audit)
  → task_completed → task_archived
```

`diff` of the two adapters' event-type sequences is empty.

### 2. Write-scope enforcement — deterministic DENY (identical)

The DENY task ingests an in-tree but out-of-`write_allow` file
(`src/other.rs`, while `write_allow = src/lib.rs`). Both adapters reject it
identically:

```
Error: Evidence rejected: file 'src/other.rs' is out of write scope or in deny list. Rule: SCOPE-001
```

DENY ledger (identical for both adapters), showing the rejection on the ledger
and the run failing closed:

```
task_created → task_marked_ready → task_started
  → lease_created → workspace_created → run_started
  → evidence_rejected → lease_revoked → workspace_cleaned → run_failed
```

The live gate agrees, for both adapters:

```
ctl hook gate --tool write --path src/other.rs  → {"allowed": false, "reason": "outside write_allow", ...}
ctl hook gate --tool write --path src/lib.rs     → {"allowed": true,  "reason": "within write_allow", ...}
```

### 3. Evidence source — the one intended divergence

The accepted evidence is tagged with the adapter that produced it; the
completion audit is tagged `completion_audit` for both:

```
omp      run:  "source":"omp"        + "source":"completion_audit"
opencode run:  "source":"opencode"   + "source":"completion_audit"
```

The `run_started` / run-manifest likewise carries `"adapter":"omp"` vs
`"adapter":"opencode"`. These are the *only* content differences between the two
HAPPY ledgers (besides per-run UUIDs and timestamps) — exactly the unambiguous
audit-trail distinction the two adapters exist to provide.

### 4. Gate binding — bound to the committed tree

After committing the fixture change, `ctl gate run cargo_check` stamps the
`gate_checked` event with `tree_hash == git rev-parse HEAD^{tree}`; the same
hash binds the completion audit. `finish` recomputes `HEAD^{tree}` and requires
the match. Both adapters: the recorded `tree_hash` equals the run's
`HEAD^{tree}` (e.g. omp `60beec61d598…`, opencode `b358392a6ff0…`).

### 5. Audit — reviewer distinct from implementer

For both adapters the completion audit is recorded by `CTL_ACTOR=ctl-review`
(the implementer cannot self-approve), and `finish` accepts it as the fresh
passing audit recorded after `submit`.

### 6. Finish / archive — identical hard-gated closure

Both adapters: `task finish` exit 0 (gates passing + fresh tree-bound audit +
clean working tree) then `task archive` exit 0.

### 7. Adapter doctor before/after — identical and stable

`ctl adapter doctor --json` captured before the run and after archive is
**byte-identical** (the governed lifecycle does not perturb adapter
diagnostics), and identical across the two adapters' workspaces.

Note the *absolute* doctor result depends on the workspace, not the run: a bare
`ctl init` temp workspace lacks the full platform wiring, so it reports
`total=2, healthy=0, failed=2` (contract checks all PASS; `platform.*` checks
FAIL/UNKNOWN because `.opencode/` and `.agent/protocols/` are absent). This
repository, which ships the full wiring, reports `total=2, healthy=0→2,
failed=0` (`pass=23, fail=0`). Either way, *before == after* — the parity claim
is about stability, not the absolute health of a throwaway workspace.

---

## Disclosure: deterministic vs model-driven deny

The write-scope DENY here is **deterministic**, not model-driven: it is produced
by handing `ctl run ingest` an `agent-output.json` whose `touched_files` names an
out-of-scope path, and by calling `ctl hook gate` directly. No language model
attempts a real out-of-scope edit in this harness.

This is intentional and is the correct level for an *adapter-parity* test: the
governance layer (scope check, ledger rejection, live gate) is exercised
identically and reproducibly for both adapters without depending on model or
provider availability. A separate, genuinely model-driven deny (a real agent
session attempting an out-of-scope write and being blocked by the host gate) is
**out of scope for this task** and would be reported separately if/when run; it
is sensitive to provider stability and is not required to establish adapter
parity.

---

## Constraints honored

- **No third platform** — only the two registered adapters (`omp`, `opencode`)
  are exercised; the harness iterates the existing registry.
- **No adapter API change** — no Rust source was modified; the harness drives
  the shipped `ctl` binary through existing commands only.
- **No model-quality benchmark** — the harness compares *governance* behavior,
  never model output or quality.
- **No scoring** — the report is pass/identical/differs facts only; no composite
  score is computed or introduced (consistent with adapter-doctor-v1).

---

## Reproduce

```bash
cargo build
bash scripts/adapter_parity_e2e.sh            # OUT_DIR printed at the end
# or pin the output location:
bash scripts/adapter_parity_e2e.sh /tmp/parity_out
```

Exit 0 means full parity. The script prints a `PARITY/DIVERGE` line per
dimension and leaves all captured artifacts (ledgers, per-step stdout, doctor
snapshots) under `OUT_DIR/<adapter>/{happy,deny}/` for inspection.
