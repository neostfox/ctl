# AI Dev Control Plane - M3 Manual Loop MVP

## Status: M3 implementation complete

### Deliverables
1. **Assignment Export** (`control assignment export --id <task>`): Generates structured assignment JSON from task boundary + context snapshot.
2. **Manual Result Ingest** (`control run ingest --id <task> --adapter manual --result <file>`): Validates touched files against write scope, accepts/rejects evidence as canonical events.
3. **Deterministic Audit Report** (`control audit --id <task>`): Generates audit report from events + evidence with completion interlock verdict.
4. **Task Report** (`control report`): Summary of all tasks.
5. **Hardened submit**: Checks hold, boundary violations, and phase before accepting submission.
6. **Hardened finish**: Full completion interlock — phase, hold, gates, rejected evidence.
7. **Evidence events**: `evidence_accepted` and `evidence_rejected` in reducer with fail-closed validation.
8. **OMP Skill**: Updated to M3 covering assignment export, evidence ingest, audit, and full lifecycle.

### Verification
Use the project gates before declaring a milestone result:

```text
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
cargo run -- architecture check
```

### M3 Command Surface

```text
control assignment export --id <task>
control run ingest --id <task> --adapter manual --result <file>
control audit --id <task>
control report
```

### Next Step
M3 dogfood: complete at least 10 real small tasks using the manual loop before considering M4.
