# Divergence — Run-Ledger Single-Writer V1 (BS-RW1)

> Originator artifact. L0 content. Task: `run-ledger-single-writer-v1`.
> Leans are not decisions; convergence (post-critic) decides.

## Problem (grounded in the code, not speculation)

The **task** ledger is single-writer-safe: `validate_and_append`
(`application/mod.rs:~1898`) holds `FileEventStore::lock_task` across the whole
read-seq → reduce/validate → append critical section, so two concurrent `ctl`
processes cannot read the same max-seq and append conflicting events. The lock is
an atomic `create_new` lock file + ownership nonce, with explicit (never
heuristic) crash recovery (`store/mod.rs:23–142`), and it has 7 tests pinning its
invariants.

The **run** ledger (M6 multi-agent runs) has **none of this**. `append_run_event`
(`application/mod.rs:~2619`) does `read_for_run → apply_run (validate) → append →
write_run_view` with **no lock**, and `build_run_event` allocates the seq
(`next_seq_for_run`) *before* the append, also unlocked. Two concurrent run
writers can read the same max-seq and both append it → duplicate seq / silently
overwritten event. `RunEventStore` (`store/run_store.rs`) has no lock method and 0
concurrency tests. This is `EPISTEMIC_CONTROL.md` §附 immediate-priority item
"事务化单写者", and it is a correctness blocker *before* concurrent multi-agent
execution — which is exactly the milestone this unblocks.

## Hard constraints (any fix must respect)

No new deps (clap/serde/anyhow only); **no OS advisory locks** (`flock`/`LockFileEx`
add a platform dep — the existing code explicitly rejects them); cross-platform incl.
Windows; append-only canonical truth; domain layer stays pure (locking is
infrastructure). The fix must not regress the task lock's tested invariants.

## Axis A — lock granularity

- **A1. Per-run lock** (mirror per-task): one `.lock` per `run_id`.
- **A2. One coarse global write lock** for all ledger writes (task + run).
- **A3. Per-ledger lock** (one lock for the whole run ledger).

Lean: **A1.** Mirrors the task ledger exactly, gives maximum concurrency (distinct
runs don't contend), and matches the per-entity model already proven. A2/A3
serialize unrelated writers needlessly and would also slow the task path.

## Axis B — reuse vs duplicate the lock mechanism

- **B1. Generalize** `TaskLock` into a shared directory-lock primitive (e.g.
  `DirLock` / `lock_dir(dir, label, timeout)`) that locks `<dir>/.lock`;
  `lock_task` delegates to it, and the run store gets `lock_run` from the same
  primitive.
- **B2. Duplicate** a separate `RunLock` in `run_store.rs`.

Lean: **B1**, *if* it can be done without regressing the task lock's 7 tests. The
lock is already "lock a directory's `.lock`" — `lock_task` only resolves
`task_dir` then locks. One audited mechanism (one nonce scheme, one Drop-safety
rule, one crash-recovery story, one test suite generalized) beats two copies that
can drift. B2's only virtue is zero risk to existing code; the critic should weigh
whether the regression risk of B1 is real given the tests.

## Axis C — what the lock must cover (the actual race)

- **C1.** Hold `lock_run(run_id)` across **read_for_run → apply_run validate →
  seq allocation → append** in `append_run_event`. This requires moving seq
  allocation (`next_seq_for_run`, currently in `build_run_event` *before* the
  append) *inside* the locked section — otherwise the lock doesn't close the
  read-seq/append race.
- **C2.** Lock only around the bare `append` (insufficient — leaves the
  read-seq/validate/append gap open).

Lean: **C1**, mirroring `validate_and_append` precisely. The seq-allocation
placement is the crux: a lock that doesn't enclose seq allocation is theater.

## Axis D — cross-ledger operations

`create_run` (`application/mod.rs:~2638`) reads task state, then appends a run
event. `start_run` reads run state. These touch two ledgers.

- **D1.** V1 scope = run-ledger single-writer only (per-run lock around run
  writes). Cross-ledger atomicity (task+run in one transaction) is **deferred** —
  V1 does not introduce a lock-ordering protocol across ledgers.
- **D2.** Also take the task lock during cross-ledger run ops.

Lean: **D1.** The blocking correctness gap is intra-run seq collision. Cross-ledger
races (e.g. a run created against a task being concurrently finished) are a
separate, lower-frequency concern; introducing multi-lock ordering now risks
deadlock and over-scopes V1. Record it as a follow-up, don't build it.

## Axis E — projections & telemetry

- `write_run_view` (run.json projection) is already atomic temp+rename, like
  `task.json`. **No lock needed** (last-writer-wins, regenerable from events).
- Telemetry (`append_telemetry`) is L1 evidence, not canonical, and intentionally
  unserialized. **Out of scope.**

Lean: leave both as-is.

## Axis F — crash recovery & timeout

- **F1.** Identical to the task lock: explicit recovery (no auto-steal), nonce
  ownership, configurable acquire timeout, error reports lock path + holder pid.

Lean: **F1.** Whatever B-decision, the run lock must inherit the *exact* safety
semantics (a live holder's lock is never stolen).

## Out of scope (carried)

Cross-ledger transactional atomicity (D2) · telemetry serialization · OS advisory
locks · async runtime · any new dependency · changing the canonical append-only
model · auto-reclaim heuristics.

## Open questions for the critic

1. **B1 vs B2:** is generalizing `TaskLock` into a shared primitive worth the risk
   to the task lock's tested invariants, or is duplicating `RunLock` the safer V1?
2. **Cross-ledger (D):** is per-run locking *sufficient* for V1, or does leaving
   `create_run`/`start_run` cross-ledger races unaddressed undermine the "single-
   writer" claim — and if deferred, what exactly remains racy and is that honest?
3. **Seq placement (C):** is moving `next_seq_for_run` inside the lock enough, or
   are there OTHER run-ledger write paths (besides `append_run_event`) that bypass
   it? (The completeness question: does *every* run write go through one seam?)
4. **Is "single-writer" even the right frame** for a per-entity (per-run) lock —
   or does the name overpromise a global guarantee the design does not provide?
5. **Test strategy:** can the task lock's concurrency tests be generalized to runs
   without weakening them, and what new race specifically must a run test exercise?
