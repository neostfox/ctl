---
name: control-guard
description: "Control plane entry point. Auto-loaded every session. Proactively routes task lifecycle — detects multi-step work, creates ctl tasks with clear boundaries, injects scope into conversation. All findings follow the Iron Law: Symptom → Source → Consequence → Remedy."
---

# Control Guard

Auto-loaded every session. You **proactively enforce** the two-layer governance model:

```
ctl task (parent)     — declared scope, gates, boundaries
  └─ OMP todo (child) — subtask tracking within parent's write_allow
```

## The Core Problem This Solves

Without control-guard: the model dives into work using OMP todo, but **no task boundaries are declared, no gates are set, no audit trail exists**. Work happens un governed.

With control-guard: **before** any multi-file change, you create a ctl task. The hook auto-injects boundaries into every conversation turn. Writes outside scope are blocked. Work happens governed, reviewed, and audited.

## Routing to specialist skills

control-guard is the router. Hand off to the right skill, don't do everything inline:

| Intent | Skill |
|---|---|
| Unclear requirements / new feature / scope a complex change | **ctl-brainstorm** (also `/ctl-new`) |
| Review a change before it lands, or audit a task before finish | **ctl-review** (read-only sub-agent) |
| A defect: gate/build/test failure, crash, recurring bug | **ctl-diagnose** |
| Generate `.ctl/spec/` from source | **ctl-spec-bootstrap** (`/ctl-spec-bootstrap`) |
| Capture a lesson into specs | **ctl-spec-update** (`/ctl-spec-update`) |

## When to Engage (PROACTIVE)

**You MUST propose creating ctl tasks when you detect:**
- Multi-file changes (2+ files to modify)
- Feature/bugfix/refactoring with a clear objective
- User describes a problem that needs investigation + code changes
- Any work that would benefit from audit trail and scope enforcement

**Skip (let the model work freely):**
- Pure conversation / Q&A
- Read-only exploration
- Trivial single-file edit (typo, comment)
- User explicitly says "skip control"

## Task Proposal Flow

When you detect task-worthy work, **you initiate** — don't wait for the user:

### Step 1: Analyze the request
1. **Read specs** — load `.ctl/spec/backend/index.md` and relevant layer specs
2. **Read codebase** — read the files that will be affected
3. **Infer boundaries** — determine read_scope, write_allow (always minimal), gates

### Step 2: Present proposal for approval

```
📋 Task Proposal: <id>

  Objective: <one sentence>

  📖 Read:    <files/dirs>
  ✏️ Write:   <files/dirs — minimal, just what needs to change>
  🚫 Deny:    <protected paths if any>
  🔍 Gates:   <gate list>
  ⚠️ Risks:   <risks>
  📋 Specs:   <which spec files were loaded>

  ✅ approve  ✏️ adjust  ❌ skip
```

### Step 3: After approval — create and start

```bash
ctl task create --id <id> --objective "<text>" \
  --read-scope <path>... --write-allow <path>... --gates <gate>...
ctl task ready --id <id>
ctl task start --id <id>
```

### Step 4: Use OMP todo for subtasks within the task

After the ctl task is active, break the work into subtasks using your **todo list**. Each subtask operates within the parent task's `write_allow`. The hook auto-injects boundaries on every turn.

### Step 5: Close the task — completion audit first

**Before finishing, run the completion audit.** Dispatch `ctl-review` (mode B) over the
whole task diff, then **record its verdict on the ledger** — this is no longer convention:
`ctl task finish` is hard-gated (M-f) and refuses to complete without a fresh passing
`completion_audit`. The audit happens in **Review** (after `submit`), the commit window is
also open in Review (M-g), so the order is: submit → record audit → commit → finish.

```bash
ctl task submit --id <id>                       # → Review (audit + commit window)
# dispatch ctl-review (mode B); then translate its VERDICT to the ledger — as the
# REVIEWER identity (M6: the implementer cannot accept its own audit):
CTL_ACTOR=ctl-review ctl review accept --id <id> --note "<health/summary>"   # VERDICT: pass
#   or, on VERDICT: fail —
CTL_ACTOR=ctl-review ctl review reject --id <id> --note "<blocking findings>" # back to fix-up
# only after a pass audit AND committing the work in scope:
ctl task finish  --id <id>                      # interlock: fresh pass + clean tree
ctl task archive --id <id>
```

A `reject` (or no audit) blocks `finish`; rework, re-`submit`, and re-audit — a prior
round's pass is stale once the task is re-submitted.

## Sub-Agent Review Protocol

Two governance gates, both mediated by the read-only `ctl-review` sub-agent. Reviewers run
as `explore` type — always spawnable (even pre-task), no write risk.

### Gate 1 — edit review (申请编辑 → 子代理审核)

Before applying an **out-of-scope edit** or a **batch of risk-bearing changes**, dispatch
`ctl-review` (mode A) on the proposed diff. The reviewer returns findings + a `VERDICT`.
Honor it: apply only on `pass` (zero 🔴 Critical). On `fail`, fix and re-review or redirect.

This is the soft form of the `/ctl-apply` primitive (the hard, gate-enforced version is on
the roadmap — see ROADMAP). Today it holds because you honor the verdict.

### Gate 2 — completion audit (任务完成 → 子代理审查)

After `ctl task submit` (in Review), dispatch `ctl-review` (mode B) over `git diff`. It runs
the closure checklist (build/test/lint **evidence**, not assertions) and emits a Health
Score + `VERDICT`. Record it with `ctl review accept|reject` (above). This gate is **hard**:
`ctl task finish` refuses without a fresh `completion_audit` pass (M-f). `fail` → back to
fix-up, re-submit, re-audit.

### Cross-task overlap check (before dispatching / editing)

Before an edit review, check whether this write collides with **another active task's**
`write_allow`. If `ctl` exposes a schedule/overlap check, use it; otherwise compare the
active tasks from `ctl hook context` yourself. On overlap, flag it in the review and
coordinate (sequence the tasks) before proceeding — concurrent writes to a shared path are
how two tasks silently corrupt each other.

### Recording verdicts (verdict → event)

A verdict is evidence, not chat. The read-only `ctl-review` sub-agent **finds and grades**
but cannot write — so **you (control-guard) record its verdict** on the ledger, **under the
reviewer's identity** (M6). Set `CTL_ACTOR` to a reviewer id distinct from the implementer:

```bash
CTL_ACTOR=ctl-review ctl review accept --id <id> --note "<Health: n; one-line summary>"  # pass
CTL_ACTOR=ctl-review ctl review reject --id <id> --note "<the blocking 🔴 findings>"      # fail
```

For the **completion audit (mode B)** this is mandatory, not advisory: `ctl task finish`
hard-blocks (M-f) until a fresh passing `completion_audit` exists (recorded after the last
`submit`). **Reviewer-lease binding (M6):** the implementer of a task **cannot** record its
own passing audit — the recording `CTL_ACTOR` must differ from whoever `start`ed/implemented
it, or `ctl review accept` is refused. (A `reject` may come from anyone, including the
implementer self-flagging.) Never hand-edit `events.jsonl`.

For **edit reviews (mode A)** the verdict stays advisory today — you honor it before
applying. The gate-enforced version is the `ctl apply` primitive (still on the roadmap).

### Dispatch constraints (injected into every sub-agent)

When you spawn any sub-agent, inject these behavioral constraints in the prompt — a
sub-agent without them will cut corners:

- **Closure discipline**: "done" requires evidence artifacts (build/test/curl output), not
  claims. "Where is the data?"
- **No speculation**: never assert a cause without tool verification.
- **Exhaust before surrender**: read the error verbatim, search, read source context, test
  the inverse hypothesis, try a fundamentally different approach — before giving up.
- Always prefix the dispatch with the active task path so the sub-agent inherits governance.

## Batch Task Creation

When the user describes a **large effort** (e.g., "rebuild all specs", "fix all P0 issues", "migrate to new framework"):

1. **Decompose** into independent parent tasks, each with **non-overlapping write_scope**
2. **Present all proposals at once**:
   ```
   📋 Batch Proposal: <effort name>

     Task 1: <id> — <objective>
       ✏️ Write: <paths>
       🔍 Gates: <gates>

     Task 2: <id> — <objective>
       ✏️ Write: <paths>
       🔍 Gates: <gates>

     ...

     ✅ approve all  ✏️ adjust  ❌ skip
   ```
3. **Create all approved tasks** in sequence
4. **Work through them one at a time** — the hook tracks which is active

### Boundary Auto-Inference Rules

| Signal | write_allow inference |
|---|---|
| Single file fix | That file only |
| Module change | `src/<module>/` |
| Cross-module refactor | Multiple `--write-allow` entries, one per module |
| Spec regeneration | `.ctl/spec/` |
| Schema change | `schemas/` + `src/domain/` |
| Test addition | `tests/` or matching source dir |

**write_allow is ALWAYS minimal.** Start narrow, widen only with explicit approval.

## Command Reference

| Action | Commands |
|---|---|
| **New task** | `ctl task create --id <id> --objective "<text>" --read-scope <path>... --write-allow <path>... --gates <gate>...` → `ctl task ready --id <id>` → `ctl task start --id <id>` |
| **Check status** | `ctl task status --id <id>` |
| **Close task** | `ctl task submit --id <id>` → `ctl review accept --id <id>` (mode-B pass) → commit → `ctl task finish --id <id>` → `ctl task archive --id <id>` |
| **Record audit verdict** | `CTL_ACTOR=ctl-review ctl review accept --id <id> --note "<summary>"` · `… ctl review reject --id <id> --note "<findings>"` (reviewer ≠ implementer, M6) |
| **Abort task** | `ctl task cancel --id <id>` |
| **Health check** | `ctl doctor` |
| **Plan / scope a task** | `ctl-brainstorm` (`/ctl-new`) |
| **Review / audit** | `ctl-review` (mode A edit · mode B completion audit) |
| **Diagnose a defect** | `ctl-diagnose` |
| **Generate specs** | `/ctl-spec-bootstrap` |
| **Update specs** | `/ctl-spec-update` |

## How the Hook Works (So You Don't Fight It)

The `.omp/hooks/pre/ctl-context.ts` hook does these automatically — **you don't need to replicate them**:

1. **`session_start` → `context`**: injects active task boundaries into every LLM call. You'll see `📋 Active ctl task boundaries` in context.
2. **`tool_call`**: blocks writes outside `write_allow` (and gates git/deps/subagent spawn) via the ctl state machine. Returns `block: true` with reason. Also tracks the per-subagent timeout.
3. **`tool_result`**: cleans up finished subagents from timeout tracking.
4. **`agent_end`**: detects spec drift, warns to regenerate.
5. **`session_shutdown`**: reminds of unfinished tasks.

**If a write is blocked**, the hook message tells you which task and which path. Don't bypass — either widen scope via `ctl task revise` or redirect your work.

## Code Quality Diagnosis

When user asks to review code, audit architecture, check tech debt, assess tests, or get a health report:

### Diagnosis Routing

| User intent | Read | Output mode |
|---|---|---|
| Review PR / diff | `decay-risks.md` + `test-decay-risks.md` | PR Review |
| Audit architecture | `decay-risks.md` (R5 focus) | Architecture Audit + Mermaid graph |
| Tech debt | `decay-risks.md` | Tech Debt Assessment |
| Test quality | `test-decay-risks.md` | Test Quality Review |
| Overall health | Both risk files | Health Dashboard (4-dimension score) |

### Iron Law

```
NEVER suggest fixes before completing risk diagnosis.
EVERY finding MUST follow: Symptom → Source → Consequence → Remedy.
```

Severity: 🔴 Critical / 🟡 Warning / 🟢 Suggestion. Health Score: Base 100, Critical −15, Warning −5, Suggestion −1. Floor 0.

## Subagent Governance

### Spawn Rules (enforced by hook)

The `task` tool is gated by the state machine:

| Subagent type | IDLE | IN_PROGRESS | COMPLETED |
|---|---|---|---|
| `explore` (read-only) | ✅ | ✅ | ✅ |
| `task` / `oracle` / `designer` | ❌ block | ✅ | ✅ |

**Reviewers are `explore`.** `ctl-review` and read-only `ctl-diagnose` runs spawn as
`explore` — always allowed, even before a task exists, with no write risk. Use them freely.

**When spawning a *writable* subagent is blocked**: create a ctl task first (`ctl task create + ready + start`), then spawn. Writable subagents inherit governance from the task ledger — their writes are gated by the same `write_allow` boundaries.

### Timeout Policy (enforced by hook)

Writable subagents have a **5-minute timeout** (configurable via `CTL_SUBAGENT_TIMEOUT_MS` env var). The hook tracks elapsed time per subagent:

- After threshold: `job poll` is **blocked** with a cancel directive.
- You MUST then: `job cancel [<id>]` and handle the work directly or re-spawn with a **smaller assignment**.

### Best Practices

- **Break large assignments** — if a subagent is writing >3 files, split into multiple smaller subagents or do it directly.
- **Use `explore` for investigation** — read-only subagents have no timeout restriction.
- **Prefer direct work for simple tasks** — spawning a subagent for a 1-file edit wastes overhead.
- **Monitor early polls** — if the first poll shows no progress after 1 minute, cancel immediately rather than waiting for the timeout.

## Error Handling

- **Write blocked**: `ctl boundary explain --path <path>`, fix path, retry. Never widen to root.
- **Task id exists**: Propose new id. Never mutate existing events.
- **Any command fails**: STOP, report error, do not skip steps.

## Anti-Patterns

- ❌ Start writing code without creating a ctl task (for multi-file work)
- ❌ Run `ctl` commands without going through this skill's flow
- ❌ Skip spec loading before proposing boundaries
- ❌ Modify files outside `write_allow`
- ❌ Manually edit `events.jsonl` or `task.json`
- ❌ Suggest a fix without diagnosing root cause (violates Iron Law)
