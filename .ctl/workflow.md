# Development Workflow

How work flows through the control plane. The model is two layers:

```
ctl task (parent)      declared objective, boundaries (write_allow), gates
  └─ todo list (child) subtasks tracked within the parent's write scope
```

control-guard is auto-loaded every session and routes you through the phases below.
Specialist skills do the heavy lifting; the `ctl` binary enforces the boundaries.

## Core principles

1. **Plan before code** — scope the task before touching files.
2. **Boundaries are injected, not remembered** — the hook injects the active task's
   write scope every turn; writes outside it are recorded by the gate.
3. **Review before it lands, audit before it finishes** — see the two review gates.
4. **Persist knowledge** — capture lessons back into `.ctl/spec/` so they survive compaction.
5. **One task at a time** — concurrent tasks must have non-overlapping `write_allow`.

---

## Phase 1: Plan

Goal: turn the request into an approved, well-scoped ctl task.

1. **Classify** the request (see `.ctl/spec/guides/complexity-classification.md`):
   - Trivial → skip the control plane, edit directly.
   - Simple → quick proposal, then `ctl task create`.
   - Moderate/Complex → load **ctl-grill-with-spec**.
2. **Align** (`ctl-grill-with-spec`, also `/ctl-new`): inspect the codebase first, ask only
   the questions the repo can't answer (one at a time, each with a recommended answer), and
   converge on a task proposal — objective, minimal `write_allow`, `read_scope`, gates, risks.
3. **Create + ready** the task:

```bash
ctl task create --id <id> --objective "<text>" \
  --read-scope <path>... --write-allow <path>... --gates <gate>...
ctl task ready --id <id>
```

`write_allow` is always minimal. For a large effort, decompose into child tasks with
**non-overlapping** write scopes (overlap forces sequential execution).

## Phase 2: Execute

Goal: implement within scope, reviewing risky changes before they land.

1. **Start** the task — this flips it to `in_progress` and opens the write window:

```bash
ctl task start --id <id>
```

2. **Implement** using a todo list for subtasks. Every write is gated against `write_allow`.
3. **Edit review (Gate 1)** — before an out-of-scope edit or a batch of risk-bearing
   changes, dispatch **ctl-review** (read-only `explore` sub-agent) on the proposed
   diff. Apply only on a `pass` verdict.
4. **Diagnose** failures with **ctl-diagnose** (gate/build/test breakage, crashes, recurring
   bugs) — grade evidence, converge a hypothesis, then fix.

## Phase 3: Finish

Goal: prove the work is done, then close the task.

1. **Completion audit (Gate 2)** — before submit, dispatch **ctl-review** over the
   whole `git diff`. It runs the closure checklist (build/test/lint **evidence**, not
   assertions) and emits a Health Score + verdict. A `fail` sends you back to fix-up.
2. **Capture lessons** — if the task revealed a pattern, gotcha, or decision, run
   **ctl-spec-update** to write it into `.ctl/spec/`.
3. **Close**:

```bash
ctl task submit --id <id>     # enters Review; gate interlock checks gates pass
ctl task finish --id <id>     # Completed (commit window opens)
ctl task archive --id <id>
```

Gates must be recorded as passing before `finish` (completion interlock). Commits are only
allowed once the task is `Completed`.

---

## Handoff

When switching sessions, agents, or before AFK:

```bash
ctl handoff export --id <task>          # read-only task snapshot (facts)
ctl handoff capture --id <task> --file judgment.json  # persist decisions + next action
```

The export gives facts (phase, gates, interlock, dirty files, events). The capture
stores agent/human judgment (decisions, uncertainties, hazards, next safe action) in
`.ctl/handoffs/<task>.json` — clearly marked as `agent_or_human_supplied`, never
canonical task state.

## Board

```bash
ctl board                    # terminal Kanban (phase columns, held markers)
ctl board --active           # non-archived tasks only
ctl board --table            # legacy table format
ctl board --json             # machine-readable
```

## Update

```bash
ctl update --merge           # sync project templates (safe merge, preserves your edits)
ctl update --merge --force   # overwrite locally modified managed files
ctl self-update              # upgrade the ctl binary
```

---

## Sub-agent protocol (OMP)

- **Reviewers and read-only investigation run as `explore`** — always spawnable (even before
  a task exists), no write risk. `ctl-review` is always `explore`.
- **Writable sub-agents inherit governance** from the active task ledger — their writes are
  gated by the same `write_allow`. They cannot be spawned without an active `in_progress`
  task.
- **Subagent timeout**: writable sub-agents have a ~5-minute budget
  (`CTL_SUBAGENT_TIMEOUT_MS`); past it, polling is blocked and you must cancel and re-scope.
- **Dispatch constraints** (inject into every sub-agent prompt): prefix with the active task
  path; require closure discipline (evidence, not claims), no unverified attribution, and
  exhaust-before-surrender. A sub-agent without these cuts corners.

## Cross-task safety

Before an edit review, check whether the write collides with **another active task's**
`write_allow` (`ctl hook context` lists active tasks; `ctl schedule plan` detects overlap).
On overlap, sequence the tasks — concurrent writes to a shared path corrupt each other.

## Command reference

| Action | Command |
|---|---|
| Plan / scope | `ctl-grill-with-spec` (`/ctl-new`) |
| New task | `ctl task create` → `ctl task ready` → `ctl task start` |
| Status | `ctl task status --id <id>` · `ctl hook context` · `ctl board` |
| Review / audit | `ctl-review` (mode A edit · mode B completion) |
| Diagnose | `ctl-diagnose` |
| Handoff | `ctl handoff export` · `ctl handoff capture` |
| Close | `ctl task submit` → `ctl task finish` → `ctl task archive` |
| Abort | `ctl task cancel --id <id>` |
| Update templates | `ctl update --merge` |
| Upgrade binary | `ctl self-update` |
| Generate specs | `/ctl-spec-bootstrap` |
| Update specs | `/ctl-spec-update` |
| Health | `ctl doctor` |
