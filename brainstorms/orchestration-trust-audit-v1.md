# Orchestration Trust Audit

**Task:** `orchestration-trust-audit-v1` (research / read-only) · ctl 0.0.3
**Scope:** fact audit only — no implementation. Verifies three capabilities
(trusted orchestrator, run attestation, authenticated reviewer principal) and
Claude Code governance parity. Every claim below is grounded in `file:line`
evidence verified against source.

## Executive Summary

- **trusted orchestrator: PARTIAL**
  ctl has a deterministic run-control substrate — run aggregate (`run_id`, task/adapter
  binding), run-scoped capability lease (task_id/adapter/scopes-bound), worktree
  isolation, disjoint-scope scheduling, single-writer locking, and a native-lease
  check at `run_started`. But it does **not** spawn/attest executors, does **not**
  issue signed run envelopes, and records **no** instruction/context/output hash,
  role, or model. It is a *run-control substrate*, not yet a *trusted orchestrator*.

- **run attestation: PARTIAL**
  Run provenance and run-scoped lease evidence exist (`run_created`, `lease_created`,
  `lease_used`, `run_started`, recovery reporting of lease status/stale/non-active).
  But `instruction_hash` / `context_hash` / `output_hash` / subagent role /
  model / provider / started_at / ended_at / exit status **do not exist** in the
  run aggregate or run event stream. Several run-scoped event arms
  (`run_finished`, `run_failed`, run-scoped `gate_checked` / `evidence_accepted` /
  `evidence_rejected`) are reducer-ready and tested but **never emitted in
  production**. This is *run-control substrate*, not *complete run attestation*.

- **authenticated reviewer principal: NOT IMPLEMENTED**
  Reviewer ≠ implementer is enforced only by comparing the `CTL_ACTOR` env-var
  string against the set of prior implementer actor strings. The actor is an
  unverified env var stamped verbatim onto events. No principal registry, no
  signature/key fingerprint/OIDC, no role policy, no signed audit events. This is
  **actor-label separation**, not an authenticated principal. The boundary is
  honestly disclosed in README/RELEASE_NOTES/DESIGN.

- **Claude Code parity: PARTIAL**
  Claude Code has full **workflow-skill + control-guard drift parity** and a
  read-only subagent role, and its hooks fail closed for Write/Edit/MultiEdit. But
  it is **not** a registered `ExecutorAdapter` (so no adapter list/status/doctor,
  no conformance suite, no same-task E2E), its **Task/subagent spawn is ungoverned**
  (OpenCode gates it), its **Bash fails open** on ctl error (OpenCode fails closed),
  and its Python hooks have **zero test coverage**. The adapter-less status is
  disclosed in code comments and (less prominently) in docs.

---

## Evidence Table

| Capability | Status | Evidence | Missing | Risk |
|---|---|---|---|---|
| Trusted orchestrator | PARTIAL | `AgentRunState` run_id/task/adapter/lease/worktree/scopes `src/domain/run.rs:60-95`; native-lease check at `run_started` `run.rs:217-236`; scheduling `application/mod.rs:3362-3381`; single-writer lock `run_store.rs:119-142` | ctl does not spawn/attest executors; no signed run envelope; no instruction/context/output hash; no role/model on run | "ctl orchestrates trusted runs" would overclaim; agents could be claimed-but-unattested |
| Run attestation | PARTIAL | run/lease events emitted in prod: `run_created` `mod.rs:3354`, `lease_created` `:3451`, `lease_used` `:3468`, `run_started` `:3479`; recover reports lease status/stale/non-active `mod.rs:3666-3733` | instruction_hash / context_hash / output_hash / role / model / provider / started_at / ended_at / exit — none exist; `run_finished`/`run_failed`/run-scoped `gate_checked`/`evidence_*` not emitted in prod (`run.rs:143-147`) | a run is a governed slot, not tamper-evident proof of *what ran against what, producing what* |
| Authenticated reviewer principal | NOT IMPLEMENTED | `actor_from_env()` reads `CTL_ACTOR`, default `"human"` `mod.rs:38-44`; no-self-review = `String` set membership `mod.rs:817-824`; stamped unsigned in `build_event` `mod.rs:2441-2451` | principal registry, signature/fingerprint/OIDC, role policy, signed audit events — all absent (`sha2` used only for content/policy hashing) | any process that sets `CTL_ACTOR=ctl-review` defeats the no-self-review interlock |
| Claude Code parity | PARTIAL | workflow + control-guard drift rows `skills.rs:603-638,710-715`; read-only `ctl-oracle` agent; fail-closed Write/Edit/MultiEdit `ctl-gate.py:15,63-79` | not in `SUPPORTED_ADAPTERS` (`adapters/mod.rs:18`); Task spawn ungoverned; Bash fail-open; no conformance/E2E; no hook tests | governance asymmetry vs OMP/OpenCode; "Claude Code is governed to parity" would overclaim |

---

## Platform Matrix

| Platform | Adapter Registry | Hook Gate | Workflow Skills | Subagent Gate | Doctor | E2E | Status / Gaps |
|---|---|---|---|---|---|---|---|
| **OMP** | ✅ `omp` (`adapters/mod.rs:18`) | ✅ TS hooks `.omp/hooks/pre/ctl-context.ts` | ✅ rows | ✅ `task` gated, explore read-only, writable inherits boundary; + timeout/job-poll `:194-209` | ✅ | ✅ | Complete platform. No dispatch hashing/record (in-memory timeout map only) |
| **OpenCode** | ✅ `opencode` | ✅ `.opencode/plugins/ctl-gate.ts` | ✅ rows | ✅ `task` in `MUTATING_TOOLS` `:51`, fail-closed `:219-227` | ✅ | ✅ | Complete platform. No dispatch hashing/record |
| **Claude Code** | ❌ `adapter: None` `skills.rs:711` | ⚠️ Write/Edit/MultiEdit fail-closed; **Bash fail-open**; **Task ungoverned** | ✅ full mirror `skills.rs:603-638` | ❌ Task not in matcher `settings.json:15` | ❌ not iterated | ❌ excluded | **Hook+skills platform, not an executor adapter**; subagent spawn ungoverned; no hook tests |

Legend: ✅ present · ⚠️ partial/asymmetric · ❌ absent.

---

## Findings

### Finding 1 — Trusted orchestrator is a run-control substrate, not a trusted orchestrator
- **Fact:** ctl creates a run aggregate with `run_id`, binds `task_id`/`adapter`,
  grants a run-scoped lease whose `scopes` must equal `write_allow` exactly,
  allocates a worktree, schedules disjoint scopes, and verifies the native lease
  (id == lease, status == Active, remaining_uses > 0) at `run_started`. It never
  spawns the executor, never signs an envelope, and records no instruction/context
  binding or output.
- **Code:** `src/domain/run.rs:60-95` (`AgentRunState` fields); `run.rs:217-236`
  (native-lease check); `run.rs:314-331` (lease scope==write_allow); start path
  `application/mod.rs:3425-3479`.
- **Why it matters:** The definition of *trusted orchestrator* requires that an
  ordinary agent cannot forge that its run was authorized, and that the dispatch
  fact (instruction/context/output) is recorded. ctl issues no envelope and binds
  no instruction/context, so authorization is procedural (the agent runs ctl),
  not cryptographic or attested.
- **Recommendation:** Classify as PARTIAL. Do not describe ctl as a "trusted
  orchestrator" yet; "deterministic run-control substrate / scheduler" is accurate.

### Finding 2 — No instruction/context/output hashing anywhere on the run or dispatch path
- **Fact:** `AgentRunState` has no hash/artifact/role/model/provider/timestamp/exit
  fields. A repo-wide search for `instruction_hash|context_hash|output_hash` finds
  exactly one hit — `application/mod.rs:1579` `"context_hashes": context_snapshot`
  — which is a **task-level assignment export** (`control.assignment.v1`) written to
  `assignment.json`, not a run event and not a subagent-dispatch record. The
  `EPISTEMIC_CONTROL.md:27,113` notes runs lack `model_id`/`context_hash` and that
  these hashes "cannot be self-written by the brainstorm skill" — i.e. acknowledged
  as unmet.
- **Code:** `src/domain/run.rs:60-95`; `application/mod.rs:1579`;
  `EPISTEMIC_CONTROL.md:27,113`.
- **Why it matters:** Without instruction/context/output hashes, a run is a
  governed *slot*, not evidence of what was actually executed. This is the core gap
  separating "run provenance" from "complete run attestation".
- **Recommendation:** Treat run attestation as PARTIAL; scope the hashes as a
  dedicated follow-up.

### Finding 3 — Production never emits several run-scoped events that the reducer handles
- **Fact:** `run_finished` (`finish_run`) is called only from `#[cfg(test)]`;
  `run_failed` (`fail_run`) has no callers at all; run-scoped `gate_checked` /
  `evidence_accepted` / `evidence_rejected` have no `build_run_event` emit site —
  production records the **task-level** equivalents instead. So a production run in
  practice carries no run-scoped gate evidence or accepted-evidence/touched_files.
- **Code:** `src/domain/run.rs:143-147` (explicit deferred-gap comment);
  emit-site audit in `application/mod.rs` (prod emits only `run_created`,
  `lease_created`, `lease_used`, `run_started`, `lease_revoked`, `run_aborted`,
  `lease_expired`).
- **Why it matters:** The run event stream looks more complete than it is; the
  "happy path" terminal/evidence events are reducer-ready but not reachable in
  production. Any claim that runs are fully attested end-to-end is currently false.
- **Recommendation:** When attestation work begins, wire `run_finished` + run-scoped
  gate/evidence emission, or document them as deferred (the comment already does).

### Finding 4 — Reviewer separation is actor-label only (NOT an authenticated principal)
- **Fact:** `actor` = `CTL_ACTOR` env var (default `"human"`), stamped unsigned onto
  every event. No-self-review compares `self.actor` against the set of `event.actor`
  strings on prior `task_started` / non-audit `evidence_accepted`. Any process that
  sets `CTL_ACTOR` to a string not in the implementer set passes the interlock.
  There is no principal registry, no signature/fingerprint/OIDC, no role policy;
  `sha2` is used only for content/policy hashing.
- **Code:** `application/mod.rs:38-44` (`actor_from_env`); `:817-824` (interlock);
  `:771-788` (`implementer_actors`); `:2441-2451` (`build_event`, unsigned);
  `domain/task.rs:327` ("unattested principal"); README.md:162 disclosure.
- **Why it matters:** The audit hard-gate (reviewer ≠ implementer) provides
  *separation of labels*, which is a real control-flow safeguard, but it is not
  identity authentication and must not be described as one.
- **Recommendation:** Conclude NOT IMPLEMENTED. Keep using the phrase
  "actor-label separation"; the docs already do this correctly.

### Finding 5 — Claude Code is a skills/hook platform, not an executor adapter
- **Fact:** `SUPPORTED_ADAPTERS = ["omp", "opencode"]`; the Claude control-guard row
  is `adapter: None`. `ctl adapter list/status/doctor`, the conformance suite, and
  the same-task E2E all iterate `supported_adapters()` and therefore exclude claude
  (the E2E doc states this explicitly). Claude *does* have full workflow-skill and
  control-guard drift parity and a read-only `ctl-oracle` agent.
- **Code:** `src/adapters/mod.rs:18`; `skills.rs:706-715` (`adapter: None`, "not an
  executor adapter"); `skills.rs:603-638` (workflow rows); `ADAPTER_PARITY_E2E.md`
  ("No third platform").
- **Why it matters:** Claude Code reaches *governance-surface* parity (skills,
  control-guard, fail-closed writes) but not *adapter* parity (no
  doctor/status/conformance/E2E). Conflating the two would overstate maturity.
- **Recommendation:** Either make Claude Code a first-class adapter **or** document
  it explicitly as a hook-only platform with a stated boundary (see Next Tasks #1).

### Finding 6 — Claude Code Task/subagent spawn is ungoverned; Bash fails open
- **Fact:** The Claude PreToolUse matcher is `Write|Edit|MultiEdit|Bash` — `Task`
  is absent, so subagent spawn hits `else: allow()` and never reaches `ctl hook
  gate`. OpenCode, by contrast, includes `task` in `MUTATING_TOOLS` and fails it
  closed. Claude excludes `Bash` from `FAIL_CLOSED_TOOLS`, so Bash **fails open** on
  ctl error; OpenCode fails Bash closed. The Claude hooks have **no test coverage**;
  the OpenCode plugin does (`ctl-gate.test.ts`). Additionally, the SessionStart
  message (`ctl-context.py:38-41`) claims mutating tools "fail closed if ctl is
  unavailable" — inaccurate for Bash and silent about the ungoverned Task tool.
- **Code:** `.claude/settings.json:15`; `.claude/hooks/ctl-gate.py:15,48-49`;
  `.opencode/plugins/ctl-gate.ts:51,74-82,219-227`; `.claude/hooks/ctl-context.py:38-41`.
- **Why it matters:** These are concrete governance asymmetries between adapters.
  The Bash fail-open and ungoverned Task are deliberate (Windows shell-parsing
  rationale; writable subagent roles deferred per `.claude/subagent-dispatch.md`),
  but they are real gaps relative to OpenCode and the SessionStart text overstates
  the boundary.
- **Recommendation:** Correct the SessionStart wording and add Claude hook tests.
  Do **not** plan to "gate the Task tool" — the U-1 spike (see Addendum, 2026-06-20)
  confirms PreToolUse cannot match Task and a subagent's inner writes are isolated
  from the parent gate; disclose this as a platform boundary instead.

### Finding 7 — Subagent dispatch is gated but never recorded as an attested fact (all platforms)
- **Fact:** In OMP and OpenCode the `task` tool is gated to an allow/deny verdict
  (read-only `explore` always allowed; writable roles require an active task; the
  spawned subagent's later writes are gated against `write_allow`). But the dispatch
  itself is **never appended to the ledger**, and **no instruction/context/output
  hash** is computed for any dispatch. OMP keeps only an in-memory spawn timestamp
  for timeout enforcement. Claude ships no writable subagent roles at all.
- **Code:** `src/cli/mod.rs:5557-5591` (verdict-only, no event append);
  `.omp/hooks/pre/ctl-context.ts:280-288` (in-memory map); `.claude/subagent-dispatch.md`.
- **Why it matters:** Dispatch is *enforced* but not *attested* — there is no
  durable record of which role/instruction/context a subagent ran with. This is the
  same substrate-vs-attestation gap as Finding 2, at the dispatch layer.
- **Recommendation:** Fold into a subagent-dispatch-attestation follow-up.

---

## Recommended Next Tasks (max 5, prioritized — design only, do not implement)

1. **`claude-code-parity-v1`** — Decide Claude Code's status explicitly: either
   register it as a first-class `ExecutorAdapter` (with doctor/status/conformance/E2E)
   **or** document it as a hook-only platform with a stated boundary. Minimum even if
   hook-only: correct the SessionStart wording (Finding 6) and add Claude hook tests.

2. **`claude-hook-tests-v1`** *(revised — superseded `claude-subagent-gate-v1`; see
   Addendum, 2026-06-20)* — ~~Gate the Claude `Task` tool through `ctl hook gate`~~
   is **infeasible**: a U-1 spike confirmed Claude Code's PreToolUse does **not**
   match the Task/Agent tool, and a spawned subagent's inner writes run in an
   isolated context the parent gate cannot reach. Claude↔OpenCode subagent-gating
   parity is therefore a **platform structural boundary, not a TODO**. Actionable
   residue: add the missing Claude **hook tests**, reconcile/disclose the Bash
   fail-open decision, and record the U-1 finding in `.claude/subagent-dispatch.md`.

3. **`subagent-dispatch-attestation-v1`** — Record the subagent dispatch as a
   canonical fact: role, adapter, parent task/run, `instruction_hash`,
   `context_hash`, `output_hash`. Closes Findings 2 and 7 at the dispatch layer.

4. **`run-attestation-emit-v1`** — Wire the reducer-ready-but-unemitted run events
   (`run_finished`, run-scoped `gate_checked` / `evidence_*`) into production, and
   add instruction/context/output hash + model/provider + started/ended/exit fields
   to the run aggregate. Promotes run provenance toward complete attestation.

5. **`authenticated-principal-v1`** — Replace actor-label-only reviewer separation
   with a verified principal identity (local keystore / signature / OIDC subject),
   a principal registry, role policy, and signed audit events. Largest scope; gate
   behind the above.

---

## Do Not Claim (yet)

README / release notes / docs must **not** use these phrases in present tense:

- "trusted orchestrator" / "ctl orchestrates trusted runs"
- "complete run attestation" / "fully attested runs" / "tamper-evident run records"
- "instruction/context/output are hashed and recorded" (for runs or dispatch)
- "authenticated reviewer" / "authenticated principal" / "independently/cryptographically verified approval"
- "signed audit events" / "signed envelopes"
- "Claude Code is a first-class / fully governed adapter"
- "Claude Code subagent spawn is gated" / "Claude Bash fails closed"
- "adapter parity across OMP, OpenCode, and Claude Code"

## Safe Claim (accurate today)

- "deterministic run-control substrate / scheduler": run_id, task/adapter-bound
  run-scoped capability lease, worktree isolation, disjoint-scope scheduling,
  single-writer locking, native-lease verification at `run_started`.
- "run provenance + lease evidence": run/lease lifecycle events emitted and
  replayable; recovery reports lease status / stale-TTL / non-active.
- "reviewer ≠ implementer by actor-label separation" (explicitly *not* an
  authenticated principal) — exactly as README.md:162 / RELEASE_NOTES.md:58-61 state.
- "subagent spawn is gated (allow/deny) in OMP and OpenCode; read-only `explore`
  always spawnable; writable roles require an active task and inherit its boundary."
- "Claude Code: full workflow-skill + control-guard drift parity, read-only
  `ctl-oracle` subagent, fail-closed Write/Edit/MultiEdit hooks — but a hook/skills
  platform, not an executor adapter (no doctor/status/conformance/E2E); Task spawn
  ungoverned **and ungovernable via PreToolUse — a platform boundary, not a TODO
  (Addendum, U-1)**; Bash fail-open; no hook tests."
- "events are integrity-hashed envelopes but not cryptographically signed; `actor`
  is a source label, not a verified identity."

---

## Minimal Follow-up Roadmap (design only — do not start until this report is reviewed)

1. `claude-code-parity-v1` — make Claude Code a first-class adapter **or** document
   it as hook-only with explicit boundary + doctor/status/e2e parity decision.
2. `subagent-dispatch-attestation-v1` — record subagent role, adapter, parent
   task/run, instruction_hash, context_hash, output_hash.
3. `authenticated-principal-v1` — replace actor-label-only reviewer separation with
   signed/verified principal identity.
4. `trusted-orchestrator-envelope-v1` — ctl issues run/subagent envelopes carrying
   scope, lease, role, budget, and hashes.

---

## Logical Fractures (whole-implementation analysis)

Beyond the four audited capabilities, the implementation carries a set of
**logical fractures** — seams where the system's vocabulary (orchestrator,
capability, attestation, canonical truth, hard gate, reviewer≠implementer)
describes a property the mechanism does not yet provide. All `file:line` verified.

### Root cause

`ctl` can do exactly one thing: compute an allow/deny verdict for a tool call a
**cooperating** host routes to it (`cmd_hook_gate`, `cli/mod.rs:5270-5605`). It
does not spawn, sign, record its verdicts, or verify identity. The strong words
describe **mandatory enforcement + cryptographic attestation + authenticated
identity**; the mechanism is **an advisory hook + structured logging + actor
labels**. Every fracture below is an instance of that gap.

### Tier A — fractures in the enforcement model itself

- **A1 — The gate records none of its verdicts; the audit trail has a hole exactly
  at the enforcement point.** Every branch of `cmd_hook_gate` is `println!` +
  `return Ok(())` — no event is ever appended (`cli/mod.rs:5270-5605`). A separate
  `cmd_hook_record_decision` (`cli/mod.rs:5607`) writes a **non-canonical**
  `.ctl/decisions.jsonl` (not a task event, not hash-chained, not covered by
  `validate`), and **no host hook calls it** (`.claude`/`.opencode`/`.omp` hooks
  invoke only `ctl hook gate` + `ctl hook context`). So every allow/deny and every
  out-of-scope attempt is invisible to any ledger. *"Evidence control plane"* vs.
  the enforcement decisions are not evidence.

- **A2 — `write_allow` governs the Write/Edit tool only; bash bypasses it.** The
  bash arm (`cli/mod.rs:5411-5555`) classifies by verb prefix (`classify_bash`,
  `5093-5144`) into commit/push/deps/build/`bash_other`; `bash_other` is allowed
  unconditionally in any governed state (`allow = !Ungoverned`, `5547`) with **no
  path check**. The `path_in_scope` + cross-task `first_overlapping_active_task`
  logic lives only in the write/edit arm (`5326-5409`). So `echo > src/x.rs`,
  `python -c`, `tee`, `cp`, `sed -i`, `Out-File` write anywhere — bypassing both
  `write_allow` and the M-c overlap guard. Sharpest case: the gate forbids Write to
  `.ctl/tasks/*/events.jsonl`, but `echo >> .ctl/tasks/x/events.jsonl` is
  `bash_other` → allowed, defeating "never hand-edit the ledger." The code admits
  it (`5113-5118`: *"not a hard security boundary"*). Live proof during this audit:
  a read-only `grep "git push"` was **denied** because `classify_bash`
  substring-matched the pattern text — over-inclusive (false deny) and
  under-inclusive (file-writing commands pass) at once.

- **A3 — "no writes without an active task" is violated in Idle via bash.** Idle:
  write/edit denied (`5400-5408`), but `bash_other` allowed (`5547`, `!Ungoverned`
  includes Idle) and `cargo_build` allowed (`5535`). The "mutation requires a
  scoped task" invariant holds for Write/Edit, not for bash. Idle is not
  write-safe.

- **A4 (root) — enforcement is cooperative, not mandatory.** The gate is a
  PreToolUse hook/plugin. Any process that does not install the hook, or writes via
  the filesystem/another tool directly, is entirely ungoverned. `.git/`,
  `write_deny`, protected paths bind only tools that route through the gate. This
  is the design ceiling — `ctl` is a convention outside the kernel, not an
  LSM/sandbox — which is why A1–A3 are the same fact, not edge cases.

### Tier B — substrate-vs-attestation (naming vs mechanism)

- **B1 — the "capability lease" is not a capability and gates no write.** The lease
  lives in the run aggregate (`run.rs:78`), checked only at `run_started`
  (`run.rs:217-236`) and `workspace_apply`; per-write enforcement uses the task
  `write_allow` (gate `5340-5347`), which never reads the lease. `scopes ==
  write_allow` is locked only at creation (`run.rs:328`); legacy runs
  (`state.lease == None`) skip the check entirely (`run.rs:215-216`). It is
  mutual-exclusion + TTL accounting, not an unforgeable presented token.

- **B2 — the run ledger is hollow in production; runs never reach Finished.**
  `finish_run` (`application/mod.rs:3486`) has only `#[cfg(test)]` callers and no
  `RunCommands::Finish`; `run_failed` has no callers at all; run-scoped
  `gate_checked`/`evidence_*` are never emitted (`run.rs:143-147`). Every successful
  production run looks, on the ledger, like it started and never completed — so
  `recover_report`/replay always see an open run lifecycle, and part of the
  cross-ledger "stranded/partial-start" classification is structural, not just
  crash recovery.

- **B3 — two parallel lease lifecycles reconciled by a repair tool.**
  `lease_created/used/expired/revoked` are reduced by BOTH the task aggregate
  (`task.rs:1212/1277/1298/1313`) and the run aggregate (`run.rs:241/348/355/362`),
  with explicit "no 2PC." The existence of `cross-ledger-detect-repair-v1` is itself
  an admission of a task↔run inconsistency window.

### Tier C — identity & dispatch

- **C1 — a HARD gate resting on a SOFT (forgeable) identity.** Finish is hard-gated
  on a fresh passing audit by actor ≠ implementer (`mod.rs:817-824`), but actor is
  the unsigned `CTL_ACTOR` env string (`38-44`, `2441-2451`) compared by
  `HashSet<String>` membership (`771-788`) — and the implementer picks both labels.
  Ceremony strength ≫ identity strength. (Honestly disclosed, but a real mismatch.)

- **C2 — read-only/writable subagent split is one hardcoded label match.**
  `is_readonly = matches!(at, "explore")` (`cli/mod.rs:5560`); `agent_type` is a
  host-supplied label. Any other read-only agent name (`ctl-oracle`,
  `claude-code-guide`) classifies as writable; a writable agent labeled `"explore"`
  classifies read-only and is always allowed. Same actor-label class, at dispatch.

- **C3 — the "dispatch read-only / writes inline" argument rests on a host
  assumption now CONFIRMED adverse (U-1, 2026-06-20).** `.claude/subagent-dispatch.md`
  flagged it unverified; the spike resolved it against us: Claude Code's PreToolUse
  does **not** match the Task/Agent tool (adding it to `settings.json:15` is inert),
  and a spawned subagent's own Write/Edit/Bash run in an isolated context that does
  **not** inherit the parent's PreToolUse gate. So "the gate is the universal choke
  point" is **structurally false on Claude** — a platform boundary, not a hole to
  patch. (OpenCode/OMP's session-level plugin *does* gate `task`; Claude's hook model
  cannot, via PreToolUse.) This makes the existing "dispatch read-only, keep writes
  inline" rule the *correct* design rather than a stopgap.

### Tier D — self-consistency / disclosure

- **D1 — the SessionStart message overstates the boundary.** `ctl-context.py:38-41`
  tells the agent mutating tools "fail closed if ctl is unavailable" — false for
  Bash (fail-open) and silent on Task (ungoverned). The system feeds the agent a
  stronger safety model than it enforces.

- **D2 — Claude is in the drift registry but not in doctor.** `adapter: None`
  (`skills.rs:711`) means `adapter doctor` never checks Claude's runtime wiring,
  yet Claude has the most runtime gaps (Bash fail-open, Task ungoverned, no hook
  tests). Drift covers skill *text*; doctor covers runtime *wiring* — and the gap
  is widest exactly where it is least observed.

- **D3 — ledger integrity is crash-integrity, not tamper-integrity.** Events are
  hashed envelopes but unsigned (`README:161`); `repair` truncates torn records;
  protection against hand-editing `events.jsonl` is gate-deny — cooperative-only and
  bypassable via bash (A2). L3 hash-chain is future.

### Disclosure judgment

| Fracture | Honestly disclosed today? | Nature |
|---|---|---|
| A4, B1, C1, D3 | ✅ yes (README:161-162, RELEASE_NOTES, `classify_bash` comment) | known trade-off — just don't escalate the wording |
| B2, B3 | ⚠️ partial (`run.rs:143-147`; repair tool implies it) | half-disclosed — ledger looks more complete than it is |
| A1, A2, A3, C3, D1, D2 | ❌ largely undisclosed | genuine implementation inconsistencies — A2/A3/D1 are the ones to address first |

### Sharpest three to address (design only — not in this task)

1. **bash write governance (A2/A3)** — either path-analyze `bash_other` / deny bash
   writes in Idle, or stop letting `write_allow` imply full coverage and state
   plainly that bash is not a write boundary. Largest live enforcement hole.
2. **fix the SessionStart wording (D1)** — one line, but it actively misinforms the
   agent about its own limits.
3. **record gate verdicts to a ledger (A1)** — even wiring the existing
   `hook record-decision` into the three host hooks would turn "what was blocked"
   into auditable evidence, which is what an "evidence control plane" implies.

---

---

## Addendum — U-1 spike resolved (2026-06-20)

A read-only `claude-code-guide` spike resolved open uncertainty **U-1** (raised
while planning the 0.0.5 fixes): *does Claude Code's PreToolUse hook fire for the
Task/subagent-spawn tool?*

**Verdict: CONFIRMED NO.** Per the official Claude Code hooks documentation,
corroborated by this repo's own `.claude/subagent-dispatch.md`:

- PreToolUse does **not** match the `Task`/`Agent`/`Skill` tools — adding `Task` to
  the `settings.json` matcher is an **inert no-op**. Agent lifecycle has a separate
  `SubagentStart` event whose deny capability is undocumented.
- A spawned subagent's own `Write`/`Edit`/`Bash` calls run in an **isolated
  context** and do **not** trigger the parent session's PreToolUse hooks; the
  subagent uses its own frontmatter hooks. `CTL_TASK_ID` propagation into a subagent
  hook environment is unspecified (assume not).

**Consequences for this report's recommendations and the 0.0.5 plan:**

1. The previously-listed `claude-subagent-gate-v1` ("gate the Claude Task tool") is
   **withdrawn as infeasible**. Claude↔OpenCode subagent-gating parity is a
   **platform structural boundary**, not unbuilt work — OpenCode's session-level
   plugin can gate `task`; Claude's PreToolUse model cannot.
2. It is **replaced** by `claude-hook-tests-v1`: add the missing Claude python hook
   tests (closes the "no hook tests" gap) and record this U-1 finding in
   `.claude/subagent-dispatch.md` as a disclosed boundary.
3. This **validates** the existing "dispatch read-only subagents, keep writes
   inline" design — subagent writes were never reachable by the gate, so inlining
   writes is the correct mitigation, not a workaround.

Source: official Claude Code hooks docs (PreToolUse tool-matching; subagents);
local `.claude/settings.json`, `.claude/hooks/`, `.claude/subagent-dispatch.md`.

---

> Per task scope, this is a read-only audit. No implementation has been performed.
> Do not start implementation until this report is reviewed.
