# Convergence — Run-Ledger Single-Writer V1 (BS-RW1)

> Convergence / Task Proposal. L0 content. Resolves the originator
> (`divergence.md`) ↔ critic (`critic.md`) delta. Task: `run-ledger-single-writer-v1`.

## Originator ↔ critic delta (what changed because of the challenge)

| # | Critic finding | Disposition | Rationale |
|---|----------------|-------------|-----------|
| Completeness | All canonical run writes funnel through `append_run_event` (the only non-test `RunEventStore::append`); the run reducer already rejects `seq <= last_seq` and dup `command_id`. C1 mechanism verified. | **CONFIRMED** | Locking `append_run_event` + moving seq allocation inside the lock closes the intra-run seq race; a stale-seq loser re-replays under the lock and is rejected. |
| Gap 1 | `start_run` does worktree creation + `run-manifest.json` write BEFORE `append_run_event`, outside any lock. | **ACCEPT** | Acquire the per-run lock at the TOP of `start_run` (and `terminate_run`), held across side-effects + append — not merely inside `append_run_event`. |
| Gap 2 | `lock_run`/`lock_dir` must `create_dir_all` the run dir and `validate_run_id` before opening `.lock` (run dir doesn't exist at `create_run` time). | **ACCEPT** | Mirror `lock_task_with`'s `create_dir_all` (`mod.rs:104`); validate the id first to keep the path-injection guard. |
| Cross-run TOCTOU | Two concurrent `start_run`s both pass `check_run_scope_overlap` then append overlapping `run_started`, violating ADAPTER-005. Per-run lock can't catch it. | **CLOSE IN V1** (user ruling) | Add a coarse **run-registry lock** held around `check_run_scope_overlap` + the `run_started` append in `start_run`, serializing concurrent starts so the overlap check is atomic with the append. |
| Naming | "single-writer" overpromises a global guarantee. | **ACCEPT** | Framed as **per-run ledger single-writer** + the registry lock for the start-overlap invariant. |
| B1 vs B2 | Generalize `TaskLock` → shared `lock_dir`, but keep the 7 task tests pinned to `lock_task` (the tripwire: if a task test must change, B1 regressed something). Thread a `label` so run errors say "run". | **ACCEPT (B1)** | Extract-and-delegate refactor; one no-steal/recovery story. Shared `LOCK_NONCE` stays (pid+counter unique across both). |
| Tests | Mirror the 7 lock tests for runs; ADD a seq-collision-rejection integration test (the untested crux of C1) and an overlap-serialization test (two concurrent starts → one rejected). | **ACCEPT** | Keep task suite unchanged; add a parallel run suite + the integration assertions. |

Defended against scope creep (critic agreed): leave `write_run_view`/`run.json` and telemetry **unlocked** (atomic temp+rename, regenerable / L1 evidence); **no OS advisory locks, no new deps**; **A1 per-run granularity** (not a global write lock).

## Converged V1 design

### Lock primitive (B1 — generalize)

Extract `FileEventStore::lock_dir(dir: &Path, label: &str, timeout) -> DirLock`
from the current `lock_task_with`: `create_dir_all(dir)` → atomic `create_new`
`<dir>/.lock` → nonce line + pid line → RAII `DirLock` whose `Drop` removes the
file iff the nonce still matches. `lock_task` becomes `lock_dir(self.task_dir(id)?,
id, …)`. Rename `TaskLock` → `DirLock` (or keep `TaskLock` as an alias). The error
text takes the `label`. **The 7 existing lock tests stay pinned to `lock_task`;
if any must change, the refactor regressed — stop.**

### Per-run lock

`RunEventStore::lock_run(run_id) -> DirLock` (or via the shared store): validate
the run_id (`validate_run_id`), `create_dir_all(.ctl/runs/<run_id>/)`, lock
`<run_dir>/.lock`. Same no-steal / explicit-recovery / configurable-timeout
semantics as the task lock.

### Coverage (where the locks are held)

- **`append_run_event`** (`application/mod.rs:~2619`): hold `lock_run(run_id)`
  across `read_for_run → apply_run validate → next_seq_for_run → append`. Move seq
  allocation inside the lock (today it is in `build_run_event`, before append).
- **`start_run` / `terminate_run`**: acquire the per-run lock at the TOP, held
  across the worktree/manifest side-effects + the append (Gap 1).
- **`start_run` additionally**: acquire the **registry lock first** (coarse,
  e.g. `lock_dir(.ctl/runs/, "run-registry", …)` on `.ctl/runs/.lock`), held
  around `check_run_scope_overlap` + the `run_started` append, so two concurrent
  starts serialize and the second sees the first's scope → rejected.
- **Lock order (deadlock-free): registry → per-run.** `create_run`/`terminate_run`
  take only the per-run lock; only `start_run` takes the registry, always before
  its per-run lock. No inverse ordering exists, so no cycle.

### Left unlocked (correct)

`write_run_view` (`run.json`, atomic temp+rename, regenerable); telemetry (L1
evidence). Cross-ledger task+run atomicity (`create_run` reading task state while
the task is concurrently mutated) is **deferred** — named in Out-of-scope.

### Tests

Parallel run-lock suite mirroring the 7 task tests (kept pinned to `lock_task`).
Plus two integration tests that are the actual crux:
1. **Seq-collision rejection**: two writers with a stale seq append under the
   lock; exactly one succeeds, the loser gets "Sequence error" (untested today for
   either ledger).
2. **Overlap serialization**: two concurrent `start_run`s with overlapping write
   scopes; exactly one starts, the other is rejected by the overlap check.

(Threaded concurrency tests can flake under CPU contention — keep them robust /
deterministic where possible.)

## Out of scope (carried + named)

- **Cross-ledger task+run transactional atomicity** — `create_run`/`start_run`
  read task state without holding the task lock; a run can be created/started
  against a task being concurrently finished/revised. Deferred; closing it needs a
  task-lock acquisition across the cross-ledger op (lock-ordering protocol).
- Telemetry serialization · OS advisory locks · async runtime · new deps ·
  changing the append-only model · auto-reclaim heuristics · tuning
  `LOCK_ACQUIRE_TIMEOUT` for FS-heavy critical sections (worktree creation under
  the lock — acceptable for V1; revisit if start latency bites).

## Recorded as follow-up

- Crash mid-`start_run` can leave a worktree + stale `.lock` + no `run_started`;
  confirm/extend recovery to cover a stale `<run_dir>/.lock` (critic U#4).

---

## Task Proposal

```
Task Proposal: run-ledger-single-writer-v1   (task_kind: implementation)
  Objective:  Give the run ledger per-run single-writer discipline (lock held across
              read->validate->seq->append, incl. start_run/terminate_run side-effects)
              AND close the cross-run start-overlap TOCTOU with a coarse run-registry
              lock, so concurrent run starts cannot collide on seq or violate ADAPTER-005.
  Read:       src, fixtures, Cargo.toml, ARCHITECTURE_GUARDRAILS.md, EPISTEMIC_CONTROL.md
  Write:      src   (store/mod.rs lock_dir extraction; store/run_store.rs lock_run;
                     application/mod.rs append_run_event/start_run/terminate_run; tests)
              brainstorms/run-ledger-single-writer-v1
  Deny:       (.git, .ctl, Cargo.toml, schemas protected) — NO schema change (no new events)
  Gates:      cargo_fmt_check, cargo_check, cargo_test, cargo_clippy
  Risks:      - concurrency-critical: must not regress the 7 task-lock tests (B1 tripwire)
              - deadlock if lock order isn't registry->per-run consistently
              - threaded tests may flake under contention (re-run, keep robust)
              - holding the run lock across git worktree creation lengthens the critical section
  Specs:      EPISTEMIC_CONTROL.md §附 (事务化单写者); ADAPTER-005; store/mod.rs TaskLock precedent
  Provenance: BS-RW1 — divergence + independent (unattested) critic + this convergence
```
