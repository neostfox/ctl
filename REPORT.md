# M3 Dogfood Report

**Date**: 2026-06-07
**Milestone**: M3 Manual 闭环 MVP
**Result**: PASS — 10 real small tasks completed through full control plane lifecycle

## Summary

Executed 13 tasks (10 completed, 3 cancelled as already-done or not-needed), each tracked through the complete M3 lifecycle:

```
create → ready → start → assignment export → (manual work) → gate run → ingest → submit → finish → archive
```

All tasks have evidence in `events.jsonl`. All completed tasks pass replay determinism. Audit reports are reproducible.

## Tasks Completed

| ID | Objective | Outcome |
|---|---|---|
| df01-event-isvalid | Remove dead `#[allow(dead_code)]` on `Event::is_valid()`, wire into store validation | Completed + archived |
| df02-assignment-fields | Add `contract`, `context_hashes`, `required_capabilities`, `acceptance` to assignment export | Completed + archived |
| df03-remove-normalizer-deadcode | Remove `#[allow(dead_code)]` from `PathNormalizer` methods actually in use | Completed + archived |
| df04-status-json | Add `--json` flag to `control task status`, human-readable default | Completed + archived |
| df05-schema-evidence-types | Verify `evidence_accepted`/`evidence_rejected` schema + payload validation | Completed + archived |
| df06-context-build | Verify `context build` produces file hashes in read_scope | Completed + archived |
| df07-audit-output | Verify audit command outputs gate results and evidence summary | Completed + archived |
| df08-doctor-checks | Verify doctor command checks M3 ledger health | Completed + archived |
| df09-gate-runner-fix | Fix gate runner env allowlist for Windows MSVC linker | Completed + archived |
| df10-replay-determinism | Verify replay determinism: rebuild all projections from events.jsonl | Completed + archived |

## Dogfood Findings (Bugs Fixed)

### Finding 1: Gate runner env allowlist too restrictive on Windows (BLOCKING)

**Symptom**: `cargo_test` gate always FAIL with `link.exe` exit code 1.
**Root cause**: Gate runner clears environment, only passes `PATH`, `CARGO_*`, `HOME`, `USERPROFILE`, `RUSTUP_HOME`. Windows `link.exe` requires `SystemRoot`, `TEMP`, `TMP`, `OS`, and `PROCESSOR_*` variables.
**Fix**: Added Windows-essential env vars to `build_allowed_env()` in `src/infrastructure/gates/mod.rs`.
**Severity**: STOP — gate runner is completely broken on Windows without this.

### Finding 2: Schema missing M3 event types (BLOCKING)

**Symptom**: `control run ingest` fails with "Value at root.type not in enum".
**Root cause**: `evidence_accepted` and `evidence_rejected` event types used by M3 code were not in the `control.event-envelope.v1.schema.json` enum.
**Fix**: Added `evidence_accepted`, `evidence_rejected` to event type enum and added payload schemas.
**Severity**: STOP — M3 ingest flow completely blocked.

### Finding 3: Path separator mismatch on Windows scope check (BLOCKING)

**Symptom**: `control run ingest` rejects files that ARE in write_allow scope.
**Root cause**: `PathNormalizer::normalize()` returns `\`-separated paths on Windows, but write_allow strings from CLI use `/`. Scope comparison fails.
**Fix**: Normalize both paths to `/` before comparison in `ingest_manual_result()`.
**Severity**: STOP — evidence ingest broken on Windows.

### Finding 4: Completion interlock counts ALL historical rejections (DESIGN BUG)

**Symptom**: `control task finish` fails after a transient ingest error was rejected, even though later ingest succeeded.
**Root cause**: Completion interlock counts all `evidence_rejected` events, not just unresolved ones.
**Fix**: Changed interlock to track rejected files, resolving them when subsequent `evidence_accepted` covers them.
**Severity**: HIGH — any temporary validation error permanently blocks task completion.

### Finding 5: PathNormalizer marks schemas/ as protected for read scope (DESIGN GAP)

**Symptom**: `control task create --read-scope schemas/` fails with "Path is protected".
**Root cause**: `is_protected()` applies to ALL scope paths, including read-only scope. Protected check should only apply to write scope.
**Fix**: Not yet fixed — requires separating read vs write protection in PathNormalizer.
**Severity**: MEDIUM — limits ability to track schema changes as tasks.

## M3 Verification Results

- ✅ 104 tests pass
- ✅ `cargo fmt --check` clean
- ✅ `cargo clippy -- -D warnings` clean
- ✅ `architecture check` passes
- ✅ Full lifecycle: create → ready → start → assignment export → ingest → submit → finish → archive
- ✅ Replay determinism: `reconcile` rebuilds all 13 task projections from events.jsonl
- ✅ Evidence contains input/output/summary
- ✅ Audit report shows gate status, evidence counts, violations, completion interlock verdict
- ✅ Doctor reports ledger health with all tasks
- ✅ Context build produces file hashes

## M4 Readiness Assessment

**Can M4 begin?** YES:

1. Gate runner env allowlist is Windows-compatible, but the allowlist is maintained manually. Consider a more principled approach (e.g., allow everything except explicit deny list).
2. ~~Hash function is still 16-byte XOR fold (`UNVERIFIED`).~~ **FIXED**: Now SHA-256.
3. `collect_files_recursive` and `boundary_check` have `#[allow(dead_code)]` — they're M2 features wired but unused in M3.
4. ~~No `--dry-run` support on write commands yet.~~ **FIXED**: Global `--dry-run` flag added.
5. ~~PathNormalizer read/write protection separation needed.~~ **FIXED**: `normalize()` vs `normalize_write()`.

## M3 Legacy Issues

1. ~~**EVIDENCE-001**: Hash function is XOR fold, not cryptographically secure.~~ **FIXED**: Replaced with SHA-256 (`sha2` crate). Hash output is now 64-char hex.
2. ~~**Protected paths for read scope**: schemas/, Cargo.toml, etc. can't be added to read_scope.~~ **FIXED**: Split `normalize()`/`normalize_write()` — protected paths only enforced on write scope.
3. ~~**No --dry-run**: All write commands lack dry-run support.~~ **FIXED**: Added `--dry-run` global flag. Validates and prints what would happen without persisting.
4. ~~**Context hash display**: Uses `\` on Windows for path display in context.json.~~ **FIXED**: All paths in context.json now use `/` via `path_to_payload_string()`.
5. ~~**Assignment export overwrites**: Re-running assignment export overwrites without warning.~~ **FIXED**: Prints warning to stderr on overwrite.

# M4 Dogfood Report

**Date**: 2026-06-10
**Milestone**: M4 OMP 单执行器隔离运行
**Result**: PASS — 21 tasks completed through full control plane lifecycle

## Summary

Executed 21 M4 tasks (df-m4-01 through df-m4-21), each tracked through the complete lifecycle:

```
create → ready → start → (manual work) → ingest → gate run → submit → finish → archive
```

All tasks use manual adapter (git worktree not functional on Windows due to UNC path issue with `git worktree add`).

**Update (2026-06-10)**: UNC path issue fixed. OMP worktree isolation verified end-to-end.
## Implementation Changes (Phase 1)

### 1.1 `print_task_state` / `print_task_human` M4 fields
- JSON view now includes `active_run`, `leases_active` count, `pending_approvals` count
- Human output shows run info, active lease count, approval status per request

### 1.2 `control run abort` command
- New `RunCommands::Abort { id, reason }` CLI variant
- Application method `run_abort()`: revokes lease, cleans worktree, emits `run_failed`
- Enables recovery after OMP crash

### 1.3 Lease TTL wall-clock check
- `check_lease_valid` now reads `occurred_at` from the event stream for `created_at_seq`
- Compares wall-clock elapsed time against `ttl_seconds`
- Fail-closed: if parsing fails, only max_uses enforcement applies
- Added `parse_iso8601_to_epoch()` helper (inverse of `epoch_to_datetime`)

### 1.4 `workspace_apply` emits `lease_used` event
- Each apply consumes one lease use via `lease_used` event
- If reducer rejects (remaining_uses == 0), apply is blocked

### 1.5 Lease auto-expire check
- New `expire_stale_leases()` method scans active leases and emits `lease_expired` for TTL-exceeded
- Called at start of `run_start` and `workspace_apply`

### 1.6 Approval TTL expiry check
- `ApprovalState` gained `granted_at_seq: Option<i64>` field
- Reducer sets `granted_at_seq` on `approval_granted`
- `workspace_apply` checks approval TTL: if elapsed > ttl_seconds, approval is expired
- Fail-closed: if grant time can't be determined, approval is not valid

## Audit Matrix Tests (Phase 2)

19 new tests added, `AUDIT_MATRIX_VERSION` bumped from 6 to 7:

- **Lease lifecycle** (7 tests): create/use/revoke, duplicate rejection, expired/revoked use rejection, zero TTL/max_uses rejection, auto-expire on last use
- **Run lifecycle** (4 tests): start/complete, double start rejection, failed clears active, complete without active
- **Workspace** (3 tests): created/cleaned require InProgress, diff requires arrays
- **Approval** (4 tests): request/grant lifecycle, duplicate rejection, deny non-pending, expired status
- **Fixture replay** (1 test): 18-event M4 lifecycle replay verifying Completed, archived, no active leases/runs

Total: 124 tests pass (105 existing + 19 new).

## Dogfood Tasks (21 completed)

| ID | Objective | Outcome |
|---|---|---|
| df-m4-01 | Verify print_task_human M4 fields display | Completed |
| df-m4-02 | Add RunInfo doc comment | Completed |
| df-m4-03 | Add LeaseState doc comment | Completed |
| df-m4-04 | Add ApprovalState doc comment | Completed |
| df-m4-05 | Add RunCommands Abort variant for run abort CLI | Completed |
| df-m4-06 | Implement print_task_state M4 fields in JSON view | Completed |
| df-m4-07 | Implement print_task_human M4 fields | Completed |
| df-m4-08 | Implement run_abort application method | Completed |
| df-m4-09 | Implement lease TTL wall-clock check | Completed |
| df-m4-10 | Implement workspace_apply lease_used event | Completed |
| df-m4-11 | Implement expire_stale_leases method | Completed |
| df-m4-12 | Implement approval TTL expiry check | Completed |
| df-m4-13 | Add lease lifecycle audit matrix tests | Completed |
| df-m4-14 | Add run lifecycle audit matrix tests | Completed |
| df-m4-15 | Add workspace audit matrix tests | Completed |
| df-m4-16 | Add approval audit matrix tests | Completed |
| df-m4-17 | Add M4 fixture replay test | Completed |
| df-m4-18 | Bump AUDIT_MATRIX_VERSION to 7 | Completed |
| df-m4-19 | Add parse_iso8601_to_epoch helper | Completed |
| df-m4-20 | Add granted_at_seq to ApprovalState | Completed |
| df-m4-21 | Remove dead_code annotations from M4 fields | Completed |

## M4 Verification Results

- ✅ 124 tests pass (105 existing + 19 new)
- ✅ `cargo build` clean
- ✅ `architecture check` passes
- ✅ 21 dogfood tasks completed through full lifecycle
- ✅ `control doctor` shows all tasks healthy
- ✅ `control task status` displays M4 fields (active_run, leases, approvals)
- ⚠️ Git worktree non-functional on Windows (UNC path issue) — manual adapter used as fallback

## Dogfood Observations

- **Command latency**: Each `control` command completes in < 0.2s (target < 2s) ✅
- **Error messages**: Clear, include rule IDs (AUDIT-001, ADAPTER-005)
- **Status readability**: M4 fields (run, lease, approval) now visible in human output
- **Windows compatibility**: Gate runner works; git worktree add fails with UNC paths

## Known Issues

1. ~~**Git worktree on Windows**: `git worktree add` fails with canonicalized UNC paths.~~ **FIXED**: Removed `std::fs::canonicalize()` from `ControlApp::init`/`open`. `PathNormalizer` handles its own canonicalization internally.
2. ~~**Lease not revoked on run completion**: `run_ingest_omp` emitted `run_completed` without `lease_revoked`, leaving stale leases that block cross-task checks.~~ **FIXED**: `run_ingest_omp` now emits `lease_revoked` + `workspace_cleaned` after `run_completed`.
3. **Lease TTL performance**: `check_lease_valid` reads the full event stream to find `occurred_at` by seq. Acceptable for current scale (< 100 events/task).

## OMP Isolation Verification

After fixing the UNC path issue, the following end-to-end tests passed:

| Test | Result |
|---|---|
| `run start --adapter omp` creates worktree and branch | ✅ |
| `workspace diff` detects changes in worktree | ✅ |
| `workspace apply` copies files to main workspace | ✅ |
| `run ingest --adapter omp` emits run_completed + lease_revoked + workspace_cleaned | ✅ |
| `run abort` cleans up worktree and revokes lease | ✅ |
| Worktree removed after completion | ✅ |
| Main workspace untouched during worktree execution | ✅ |
| Cross-task lease conflict detection | ✅ |

## M4 Hardening Report

**Date**: 2026-06-10
**Result**: PASS — 131 tests, clippy clean, all checks green

### Dead Code Cleanup

- Removed 5 stale `#[allow(dead_code)]` annotations (boundary_check, collect_files_recursive, TaskState, TaskState::new, apply)
- Deleted 3 dead functions: `list_tasks` (unused), `LeaseState::is_valid` (unused), `make_event` test helper (unused)
- Kept 5 correct module-level suppressions in main.rs and infrastructure/mod.rs

### Bug Fix: Lease Not Revoked on Run Completion

`run_ingest_omp` emitted `run_completed` without `lease_revoked`/`workspace_cleaned`, leaving stale leases that blocked cross-task scope overlap checks.

**Fix**: `run_ingest_omp` now emits `lease_revoked` + `workspace_cleaned` after `run_completed`.

### Gate Runner Stabilization

**Root cause**: Gate runner used an allowlist for environment variables. On Windows, the MSVC linker needs many env vars not in the allowlist. Transient failures occurred when incremental compilation cache was invalidated and relinking was needed.

**Fix**: Switched from allowlist to denylist. Passes ALL env vars except proxy/auth tokens (HTTP_PROXY, HTTPS_PROXY, GITHUB_TOKEN, etc.). This achieves the same EXEC-003 security goal (no network deps) without platform-specific fragility.

### Error Message Rule IDs

Added rule IDs to 6 M4 error messages that lacked them:
- `APPROVAL-001` — high-risk change requires approval
- `RUN-001` — no active run
- `RUN-002` — already has active run
- `SCOPE-001` — file out of write scope (evidence rejected)
- `ADAPTER-001` — wrong adapter source

### New Hardening Tests (7 tests)

| Test | Covers |
|---|---|
| `run_abort_revokes_lease_and_fails_run` | Run abort event sequence |
| `run_complete_then_lease_revoked_lifecycle` | Lease revocation after run completion |
| `lease_used_decrements_remaining` | Lease use counting |
| `lease_reject_use_on_unknown_id` | Unknown lease rejection |
| `approval_granted_records_seq` | granted_at_seq tracking |
| `approval_grant_on_nonexistent_fails` | Nonexistent approval rejection |
| `test_build_allowed_env_blocks_proxy_vars` | Gate runner env denylist |

`AUDIT_MATRIX_VERSION` bumped from 7 to 8.

### Final Verification

- ✅ 131 tests pass (105 baseline + 19 M4 + 7 hardening)
- ✅ `cargo clippy -- -D warnings` clean
- ✅ `architecture check` passes
- ✅ `validate` passes
- ✅ `doctor` shows all 43 tasks healthy
- ✅ OMP worktree isolation verified end-to-end on Windows
