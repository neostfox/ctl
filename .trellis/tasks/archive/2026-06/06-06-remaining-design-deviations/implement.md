# Implementation Plan: Resolve Remaining Design Deviations

Implementation must not start until the planning questions in `prd.md` are resolved and the user approves proceeding.

## Proposed Task Split

### Parent Task: Resolve remaining design deviations

Owns cross-milestone decisions and integration review.

### Child 1: M1 Canonical Task Ledger

Goal: Restore local task ledger commands on the chosen canonical store path.

Scope:

- Store path decision implementation.
- `task_created` / `task_revised` schema upgrade.
- Reducer state update for structured boundaries.
- Projection schema and writer update.
- CLI commands: `init`, `task create`, `task revise`, `task ready`, `task status`, `replay`, `validate`, `doctor` only if their contracts are covered.

Acceptance:

- `events.jsonl` is append-only canonical truth.
- `task.json` is rebuildable by replay.
- Empty or invalid boundaries fail before append.
- Architecture check fails on dual canonical stores.

### Child 2: M2 Boundary and Gate Policy

Goal: Reintroduce gate templates and runner only with EXEC-002 controls.

Scope:

- Gate template schema.
- Runner policy: env allowlist, timeout, output cap, redaction, deterministic command templates.
- Gate result event semantics.
- Protected path and scope-diff checks.

Acceptance:

- `gate run` either passes all EXEC-002 requirements or remains disabled.
- Unknown gates fail schema/app/reducer checks.
- Required gate baseline changes require architecture review.

### Child 3: M3 Manual Loop

Goal: Prove adapter-neutral manual execution before any automated adapter.

Scope:

- Assignment export schema.
- Manual result ingest schema.
- Evidence acceptance/rejection events.
- Deterministic audit report.
- `task submit` / `task finish` only after completion interlock is complete.

Acceptance:

- External executors cannot append canonical events directly.
- Manual output is evidence until the control layer validates it.
- `task finish` fails closed on missing gates, out-of-scope touched files, missing evidence, pending approvals, active holds, or baseline regression.

## Ordered First Implementation Checklist After Approval

1. Finalize canonical store path decision.
2. Finalize task boundary contract decision.
3. Update schema docs and fixtures for the selected M1 contract.
4. Refactor store path and projection writer.
5. Update reducer to persist structured boundaries.
6. Reintroduce only M1-safe CLI commands.
7. Extend architecture checks for canonical store, boundary contract, and M1 command scope.
8. Add focused tests for invalid paths, empty boundaries, projection replay, and command surface.
9. Run verification gates.

## Validation Commands

```text
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
cargo run -- architecture check
```

Focused probes to add/run during M1:

```text
schema validate rejects empty read_scope/write_allow/gates
boundary rejects canonical events paths as write scope
replay regenerates identical task.json from same event stream
task create rejects protected path, unknown gate, empty objective
task ready rejects incomplete task definition
```

## Risky Files / Rollback Points

- `schemas/control.event-envelope.v1.schema.json`
- `schemas/control.task-definition.v1.schema.json`
- `schemas/control.task-view.v1.schema.json`
- `src/domain/task.rs`
- `src/infrastructure/store/mod.rs`
- `src/infrastructure/boundary/normalizer.rs`
- `src/cli/mod.rs`
- `fixtures/*.jsonl`
- `src/domain/audit_matrix.rs`

Rollback rule: do not leave schema and reducer on different contracts. If a schema migration fails, revert schema, fixtures, reducer, and architecture check together.

## Pre-Start Review Gate

Before `task.py start`:

- User approves canonical store path.
- User approves structured task boundary model.
- This parent task is either kept as planning-only or split into child implementation tasks.
