# Known Defect — `cargo_test` gate was flaky (RESOLVED)

> Status: **resolved** by task `make-cargo-test-gate-reliable` (2026-07-12).
> Root cause confirmed and fixed. Kept as a knowledge note for future
> reference. Record-only; gates nothing.

## Symptom (was)

The `cargo_test` gate intermittently reported **FAIL (exit=101)** while the
identical `cargo test` run manually always passed. Observed at 2/4 during
back-to-back gate loops; 0/N in a plain shell.

## Root cause (confirmed)

Two distinct defects, both fixed:

1. **The flake itself — a Windows file-lock race in `lock_dir`.** The failing
   test was `infrastructure::store::run_store::tests::concurrent_locked_appends_get_contiguous_seqs`
   (4 threads locking/unlocking `.ctl/runs/run-x/.lock` in a tight loop).
   `lock_dir` (`src/infrastructure/store/mod.rs`) acquires the lock with
   `OpenOptions::create_new(true)` and retried **only** `AlreadyExists`. On
   Windows, a concurrent `create_new` racing another process's `remove_file`
   (the `Drop` of a just-released lock) transiently surfaces as
   `ERROR_ACCESS_DENIED` (os error 5) — a known `CreateFile(CREATE_NEW)` quirk —
   instead of `ERROR_FILE_EXISTS`. That hit the fatal `Err(e) =>` arm and
   propagated as an unexpected `Err`, so the test's `.unwrap()` panicked. (This
   is the `make-cargo-test-gate-reliable` board's pinned root cause — NOT the
   `supervise_timeout_*` timing hypothesis originally ranked first.)

   **Fix:** `is_lock_contention()` now also treats Windows `ERROR_ACCESS_DENIED`
   (raw os error 5) as retryable; `lock_dir` retries it within `acquire_timeout`
   like `AlreadyExists`. Scoped to Windows (on Unix, `PermissionDenied` on
   `create_new` is almost always a genuine access problem, not a create-race
   quirk). Verified: 20× back-to-back `cargo_test` gate runs all PASS (was
   failing ~1-in-12).

2. **The diagnosability gap that hid #1.** `run_gate_checked`
   (`src/application/mod.rs`) formatted failed-gate evidence as
   `exit=<n> stderr=<preview>` — but cargo writes the failing-test **name + panic
   to stdout**, which was discarded. So a failed `cargo_test` gate showed only
   `exit=101 stderr=error: test failed, to rerun pass --bin ctl` with no test
   name, making the flake un-diagnosable from the ledger.

   **Fix:** extracted pure `format_gate_evidence()` + `truncate_preview()` /
   `truncate_tail_preview()`. Failed-gate evidence now includes a **stdout TAIL**
   preview (1024 B, char-boundary safe) — cargo writes `test result: FAILED` +
   the failing-test panic at the END of stdout, so a head window buried it under
   hundreds of `… ok` lines; the tail window surfaces the failure identity.

## How the root cause was pinned

The stdout-tail evidence (#2) is what made #1 findable: after rebuilding, a gate
loop caught a failure whose evidence now read
`…concurrent_locked_appends_get_contiguous_seqs stdout ---- panicked at run_store.rs:516 … failed to acquire run lock … 拒绝访问。 (os error 5)`,
naming both the test and the exact error.

## Verification

- `cargo test --bin ctl`: 546 passed / 0 failed.
- New unit tests: `is_lock_contention_treats_windows_access_denied_as_retryable`
  (store), `format_gate_evidence_failed_gate_includes_stdout_preview`,
  `format_gate_evidence_failed_stdout_keeps_tail_not_head`,
  `truncate_preview_never_splits_a_char_boundary`,
  `truncate_tail_preview_is_char_boundary_safe`,
  `format_gate_evidence_passed_and_timed_out_shapes` (application).
- Empirical: 20× back-to-back `cargo_test` gate runs via the rebuilt binary —
  ALL PASS (previously failed at run ~11–12).

## Note

The fix ships when the `ctl` binary is next published (`@velo-ai/ctl`) — until
then the installed binary still records the old (stderr-only) evidence, though
the `cargo test` it runs compiles the fixed source and so no longer flakes.
