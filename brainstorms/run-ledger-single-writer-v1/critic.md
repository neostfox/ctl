# Critic — Run-Ledger Single-Writer V1

> Independent adversarial review of `brainstorms/run-ledger-single-writer-v1/divergence.md`.
> Grounded in `src/infrastructure/store/mod.rs`, `src/infrastructure/store/run_store.rs`, `src/application/mod.rs`, `ARCHITECTURE_GUARDRAILS.md`. Produced in a separate context; independence is **unattested** (no trusted orchestrator). Leans below are positions, not rubber stamps.

## Verdict in one line

The core diagnosis is correct and the C1 fix is sound, but the divergence has **two real gaps** (an unlocked pre-append side-effect in `start_run`, and a lock-key/directory-existence subtlety) and **one naming honesty problem** ("single-writer" overpromises). Fixable before convergence; do not converge as-written.

## Completeness — does every run write funnel through one locked seam? (Q3, the most important)

I enumerated every run-ledger canonical write path:

- `create_run` → `build_run_event` → `append_run_event` (`application/mod.rs:2664-2665`)
- `start_run` → `build_run_event` → `append_run_event` (`application/mod.rs:2742-2743`)
- `terminate_run` (shared by `finish_run`/`fail_run`/`abort_run`) → `build_run_event` → `append_run_event` (`application/mod.rs:2792-2793`)

The only `RunEventStore::append` call in non-test code is inside `append_run_event` itself (`application/mod.rs:2630`). **Confirmed: every canonical run-event write funnels through the one seam.** So locking `append_run_event` + moving `next_seq_for_run` inside the lock *does* close the intra-run seq-collision race for canonical events. The run reducer already has the exact guard that makes this work: `apply_run` rejects `event.seq <= state.last_seq` with "Sequence error" (`domain/run.rs:129-134`) and skips processed `command_id`s (`domain/run.rs:124-126`) — identical to the task path. A stale-seq loser re-replays under the lock and is rejected, not silently appended. **The C1 mechanism is verified correct.**

**However — two non-canonical run writes are NOT under the proposed lock**, and the divergence is silent on the first:

1. **`run-manifest.json` write in `start_run` (`application/mod.rs:2730-2735`) happens BEFORE `append_run_event`, outside any lock.** It is temp+rename atomic (so no torn file), and it is L1/derived not canonical — so last-writer-wins is *probably* tolerable like `run.json`. But two concurrent `start_run` calls on the *same* run_id is a nonsense state the V1 lock would otherwise prevent for the event, yet here the manifest write races freely and `create_run_worktree` (2720) runs unguarded too. **Finding (medium):** the divergence's Axis E only reasons about `write_run_view`/telemetry; it never accounts for the manifest + worktree side-effects in `start_run`. These execute *before* the lock would even be taken under the proposed `append_run_event`-only locking. If the intent is "a started run is single-writer," the lock must be acquired in `start_run` *around the side-effects too*, not merely inside `append_run_event`. **Recommend:** state explicitly whether `start_run`'s pre-append side-effects are in or out of scope. My position: acquire the run lock at the *top* of `start_run` (and `terminate_run`) and hold it across side-effect + append. At minimum, document the residual.

2. `write_run_view` (`run_store.rs:130`) — temp+rename, regenerable; leaving it unlocked (Axis E) is **correct** (defended below).

## Q1 — B1 (generalize) vs B2 (duplicate)

**Position: B1, but only via extract-and-delegate, and the 7 tests must stay pinned to `lock_task`.** The task lock is *already* "lock a directory's `.lock`": `lock_task_with` resolves `task_dir`, `create_dir_all`s it, then locks `task_dir/.lock` (`mod.rs:102-105`). A `lock_dir(dir: &Path, label: &str, timeout)` extraction is mechanical: `lock_task` becomes `self.lock_dir(self.task_dir(id)?, id, t)`. Concrete regression risks to call out before doing it:

- The error message embeds `task '{}'` (`mod.rs:122`) and the timeout text. Generalizing must thread a `label` so the run error says "run" not "task" — otherwise the run-lock UX lies. Don't let `lock_dir` hardcode "task".
- `LOCK_NONCE` is a single process-global `AtomicU64` (`mod.rs:16`); shared by task and run locks it is still unique (pid+counter), so **no collision risk** — good, keep it shared, one nonce scheme.
- The `Drop` nonce-check (`mod.rs:43-44`) and the no-steal loop must be *byte-identical*; the 7 tests (`mod.rs:509-599`) must continue to exercise `lock_task` (the public API), so extraction is validated by the *unchanged* suite. If a test has to change to accommodate the extraction, B1 has regressed something — that is the tripwire.

B2's only merit is zero blast radius. Given the tests pin behavior and the extraction is a pure refactor, **B1's regression risk is low and the anti-drift payoff (one no-steal story, one recovery story) is real.** Take B1.

## Q2 / Axis D — the residual cross-ledger race, and is "single-writer" honest?

`create_run` reads task state via `replay_task` (`application/mod.rs:2639`) then appends a run event — **the task read and the run append are not jointly atomic, and the task is not locked.** Concrete residual race left open by V1:

- T1 calls `create_run("t", "omp")`: reads task `t` as `InProgress` with `write_allow=[src]` (passes the guard at 2640-2652), then proceeds to append `run_created`.
- Concurrently T2 finishes/cancels task `t` (or revises its scope). The run is now created against a task state that no longer holds — a queued run inheriting a write scope the task has since dropped, or attached to a completed task.

Same shape for `start_run`: `check_run_scope_overlap` (`application/mod.rs:2705`, `2672-2691`) reads `active_runs()` (replays *other* runs) and then appends `run_started`. **Two runs starting concurrently can both pass the disjoint-scope check against a snapshot that doesn't yet include the other**, then both append `run_started` with overlapping scopes — defeating the M6 core invariant (`ADAPTER-005`: 重叠写 scope 仍然拒绝). A per-run lock does **not** close this: the two runs have different run_ids, so they take different locks and never contend. **This is the most dangerous residual** — more than the create-run/finish-task race — because it silently violates the disjoint-write-scope guarantee that the whole milestone rests on.

**Honesty judgment: calling V1 "single-writer" is defensible for the *intra-run* ledger but the divergence must explicitly disclaim the cross-run scope-overlap TOCTOU**, because a reader will assume "single-writer for runs" implies the overlap invariant is safe under concurrency. It is not. **Recommended wording:** rename the deliverable framing to **"Per-Run Ledger Single-Writer V1"** and add to Out-of-scope a named item: *"Cross-run start-overlap is racy: two concurrent `start_run`s can both pass `check_run_scope_overlap` and append overlapping `run_started`. V1 does not serialize the overlap check; closing it needs a shared run-registry lock (Axis D2 / a coarse `lock_run("__active__")` around the check+append)."* Without that disclaimer, V1 overpromises.

## Q4 — is "single-writer" the right frame for a per-entity lock?

**Partially. The name is fine for the ledger-per-run guarantee but is read as a global guarantee it does not give.** The task lock has the same per-entity shape and nobody is misled because tasks don't have a cross-entity invariant gated on a concurrent read. Runs *do* (scope-overlap). So the per-run frame is right for storage integrity, wrong as a stand-in for M6 concurrency safety. Use "per-run ledger single-writer" and keep the overlap problem as an explicit, separately-tracked follow-up.

## Lock key and lock-file location — is `run_id`/`<run_dir>/.lock` correct?

Mostly yes, with one wrinkle the divergence glosses. A run **does** get a stable directory `.ctl/runs/<run_id>/` (`run_store.rs:26`), so `<run_dir>/.lock` mirrors `<task_dir>/.lock` cleanly, and `run_id` (a UUID from `generate_uuid()`, `application/mod.rs:2656`) is a stable, collision-free key. **But:** at `create_run` time the run directory **does not yet exist** — it is created lazily by `append`/`write_run_view` (`run_store.rs:48,148`). `lock_task_with` handles exactly this by calling `create_dir_all(&task_dir)` *before* opening `.lock` (`mod.rs:104`). The generalized `lock_dir` / `lock_run` **must replicate that `create_dir_all` step**, or the very first `create_run` lock acquire will fail (no parent dir). This is a concrete must-do, not optional. Also note `validate_run_id` (`run_store.rs:159-172`) is stricter than `validate_task_id` — `lock_run` must validate the run_id before joining, to keep the path-injection guard the task path has.

## Q5 — test strategy

The 7 task tests (`mod.rs:509-599`) generalize 1:1 to runs (acquire/release/reacquire, times-out-while-held, blocks-then-succeeds, live-lock-never-stolen, drop-doesn't-delete-foreign, explicit-recovery). Keep them pinned to `lock_task` and add a *parallel* run suite — do not rewrite the task suite to be generic (that would weaken the regression value). **The new race a run test MUST exercise that the task tests do not:** a concurrency test where two threads each build with a *stale* seq and then append under the lock, asserting exactly one append succeeds and the loser gets "Sequence error" — i.e. prove the seq-allocation-inside-lock actually rejects the duplicate, not just that the lock blocks. The existing task suite tests the *lock* but not the *seq-collision rejection through the locked append*; that integration assertion is the crux of C1 and is currently untested for either ledger.

## Concrete changes required before convergence

1. **Acquire the run lock in `start_run`/`terminate_run` around the FS side-effects, not just inside `append_run_event`** (or explicitly scope-out the manifest/worktree race). As written, locking only `append_run_event` leaves `create_run_worktree` + manifest write (`application/mod.rs:2720-2735`) racing.
2. **`lock_run`/`lock_dir` must `create_dir_all` the run dir and `validate_run_id` before opening `.lock`** — the run dir doesn't exist at `create_run` time (`run_store.rs:48`), unlike the always-resolvable task dir.
3. **Disclose the cross-run `start_run` scope-overlap TOCTOU** in Out-of-scope with the precise mechanism and the future fix (coarse run-registry lock), and rename to "per-run." Add the seq-collision-rejection integration test (Q5).

## What the originator got RIGHT (defend against scope creep)

- **Axis E: leaving `write_run_view` and telemetry unlocked is correct.** `run.json` is temp+rename atomic (`run_store.rs:150-153`) and regenerable from events per `STATE-002`; serializing it buys nothing. Telemetry is L1 evidence (`STATE-003`). Do not let the critique above tempt anyone into locking these.
- **No OS advisory locks / no new deps** is correctly carried and matches the explicit rejection in `mod.rs:29-31` and `DEP-002`. The `create_new` + nonce mechanism is the right primitive; don't reach for `flock`.
- **A1 (per-run lock) over A2/A3** is the right granularity for the *ledger* race — A2/A3 would needlessly serialize the proven task path.

## Revised uncertainty set (open unknowns I still see)

1. **Is the cross-run scope-overlap TOCTOU acceptable to defer for M6?** `ADAPTER-005` is a STOP rule ("重叠写 scope 仍然拒绝"). If concurrent `start_run` can violate it, deferring may itself breach a guardrail — needs a human ruling, not an originator lean.
2. **Should `start_run`'s lock span the worktree creation**, given worktree creation is non-idempotent and expensive? Holding a 10s-timeout lock across a `git worktree add` may be fine, but the acquire-timeout (`LOCK_ACQUIRE_TIMEOUT`, `mod.rs:13`) was tuned for fast append, not for FS-heavy critical sections.
3. **Does any CLI path call `create_run`/`start_run`/`terminate_run` in a loop or batch** (where lock churn or partial failure matters)? Not audited beyond the single-call sites.
4. **Crash-recovery interaction:** a crashed `start_run` could leave a worktree + no `run_started` event AND (post-fix) a stale `.lock`. A run stuck Queued-with-worktree after a crash mid-`start_run` may be invisible to recovery. The lock adds a second stale artifact to reclaim — confirm the recovery story covers stale `.lock` under `.ctl/runs/<id>/`.
5. **`run_store()` is re-`init`'d on every call** (`application/mod.rs:2553-2555`), so `append_run_event` opens the store twice. Harmless today, but if the lock guard lives on a `RunEventStore` instance, the lock and the append must use the *same* resolved paths — verify the two `init`s can't disagree.
