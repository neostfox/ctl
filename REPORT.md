# AI Dev Control Plane - M0 Review Report

## Status: M0 boundary protocol implementation

### Deliverables
1. **Rust CLI Skeleton**: M0 surface is limited to schema, boundary, and architecture checks.
2. **JSON Schemas**:
   - `control.event-envelope.v1.schema.json`
   - `control.task-definition.v1.schema.json`
   - `control.task-view.v1.schema.json`
   - `control.policy-decision.v1.schema.json`
3. **Domain Logic**: Pure reducer for task state transitions.
4. **Path Normalization**: Windows-aware canonicalization and protected-path rejection.
5. **Fixtures**: Reducer lifecycle, hold, revise, and schema counter-example fixtures.
6. **Audit Matrix**: Unit tests pin schema, fixture, required-gate, and transition coverage.

### Verification
Use the project gates before declaring a milestone result:

```text
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
cargo run -- architecture check
```

### Next Step
Proceed to M1 only after the M0 gates above pass on the current tree.
