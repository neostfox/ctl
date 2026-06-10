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
