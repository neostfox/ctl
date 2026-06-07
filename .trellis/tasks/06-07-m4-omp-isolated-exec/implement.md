# M4 Implementation Plan

## Phase 1: Foundation (domain + infrastructure)

### 1.1 Schema additions
- Add 15 new event types to `schemas/control.event-envelope.v1.schema.json`
- Add payload schemas for each
- **Files**: `schemas/control.event-envelope.v1.schema.json`
- **Validate**: `cargo test schema_rejects_unknown_event_type`

### 1.2 Domain: Lease + Approval value objects
- `src/domain/lease.rs` — LeaseState, LeaseEvent (created/used/expired/revoked)
- `src/domain/approval.rs` — ApprovalState, ApprovalRequest, ApprovalEvent (requested/granted/denied/expired)
- **Files**: `src/domain/lease.rs`, `src/domain/approval.rs`
- **Validate**: `cargo check`

### 1.3 TaskState extensions
- Add to TaskState: `active_run: Option<RunInfo>`, `leases: HashMap<String, LeaseState>`, `pending_approvals: HashMap<String, ApprovalState>`
- Extend reducer to handle new event types
- **Files**: `src/domain/task.rs`
- **Validate**: `cargo test`

### 1.4 Adapter trait
- Define `ExecutorAdapter` trait in `src/adapters/mod.rs`
- Refactor manual adapter to implement trait
- **Files**: `src/adapters/mod.rs`, `src/adapters/manual/mod.rs`
- **Validate**: `cargo check`

## Phase 2: Workspace (git worktree management)

### 2.1 Infrastructure: workspace module
- `src/infrastructure/workspace/mod.rs`
- `create(task_id)` — git worktree create
- `diff(task_id)` — git diff HEAD vs worktree
- `apply(task_id)` — copy files from worktree to main
- `cleanup(task_id)` — remove worktree
- **Files**: `src/infrastructure/workspace/mod.rs`
- **Validate**: `cargo check`

### 2.2 Application: workspace commands
- Wire workspace commands into ControlApp
- `workspace_create` — create worktree + record event
- `workspace_diff` — compute diff + detect high-risk changes
- `workspace_apply` — apply with scope + approval checks
- **Files**: `src/application/mod.rs`
- **Validate**: `cargo test`

### 2.3 High-risk detection
- Simple rule-based detection: deletion, Cargo.toml/Cargo.lock changes, protected paths
- Generate approval requests for high-risk diffs
- **Files**: `src/application/mod.rs`
- **Validate**: `cargo test`

## Phase 3: Lease + Approval

### 3.1 Lease management
- `lease_create` — create lease with TTL + max_uses
- `lease_check` — validate lease before each operation
- `lease_use` — decrement max_uses
- `lease_expire` — check TTL on access
- All lease state from events (event sourcing)
- **Files**: `src/application/mod.rs`
- **Validate**: `cargo test`

### 3.2 Approval management
- `approval_request` — create structured request with TTL
- `approval_grant` — approve request
- `approval_deny` — reject request
- `approval_check_expiry` — check TTL on access
- **Files**: `src/application/mod.rs`
- **Validate**: `cargo test`

## Phase 4: OMP Adapter

### 4.1 OMP adapter implementation
- `src/adapters/omp/mod.rs`
- `capabilities()` — report adapter capabilities
- `prepare_run()` — generate run manifest
- `validate_output()` — validate agent-output.json
- **Files**: `src/adapters/omp/mod.rs`
- **Validate**: `cargo check`

### 4.2 Run lifecycle
- `run_start` — create worktree + lease + manifest + record event
- `run_ingest` — validate output + workspace diff + approval checks
- Run manifest generation
- **Files**: `src/application/mod.rs`
- **Validate**: `cargo test`

## Phase 5: CLI

### 5.1 New CLI commands
- `control adapter capabilities omp`
- `control workspace create/diff/apply`
- `control run start --adapter omp`
- `control approval request/grant/deny`
- **Files**: `src/cli/mod.rs`
- **Validate**: `cargo check`, `cargo test`

### 5.2 OMP skill update
- Update `.omp/skills/control-guard/SKILL.md` for M4 workflow
- **Files**: `.omp/skills/control-guard/SKILL.md`

## Phase 6: Fixtures + Tests

### 6.1 Domain fixtures
- `fixtures/lease_lifecycle.jsonl`
- `fixtures/approval_lifecycle.jsonl`
- `fixtures/workspace_diff.json`
- **Validate**: `cargo test`

### 6.2 Integration tests
- Full M4 lifecycle test: create → workspace → run start → diff → approval → apply → ingest → finish
- Lease failure cases: expired, cross-task, duplicate, max_uses exceeded
- Approval failure cases: unapproved high-risk, expired approval
- **Validate**: `cargo test`

## Phase 7: Dogfood

### 7.1 M4 dogfood
- Execute 10 real tasks through OMP adapter
- Exercise: workspace create, diff, apply, approval flow, lease management
- Record findings
- **Validate**: `cargo test`, `cargo clippy -D warnings`, `architecture check`

## Risky Files / Rollback Points

- `src/domain/task.rs` — reducer changes affect all existing tests
- `schemas/control.event-envelope.v1.schema.json` — schema changes break validation
- `src/application/mod.rs` — core application logic

## Validation Commands

```bash
cargo fmt --check
cargo check --locked --offline
cargo test --locked --offline
cargo clippy --locked --offline -- -D warnings
cargo run --locked --offline -- architecture check
```
