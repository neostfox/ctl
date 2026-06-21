---
name: control-guard
description: "Control plane entry point (Claude Code). Proactively routes ctl task lifecycle — scope, gates, audit, finish — while the .claude PreToolUse hook enforces boundaries and injects context. Read-only work dispatches to subagents; writes stay inline. Findings follow the Iron Law: Symptom → Source → Consequence → Remedy."
---

# Control Guard (Claude Code)

Auto-loaded every session. The **managed core** below is the platform-neutral
control-guard protocol, byte-checked by CI against
`.agent/protocols/control-guard.md` and the OMP / OpenCode skills. Do not edit it
here in isolation. Claude-specific mechanics live in "Claude Code Integration"
after the core.

<!-- ctl:control-guard-core:start version=1 -->
# Control Guard — Core Protocol

CONTROL_GUARD_PROTOCOL_VERSION = 1

This is the platform-neutral control-guard protocol. It is embedded **verbatim**
inside each platform skill's managed-core block; the canonical copy lives at
`.agent/protocols/control-guard.md`. A CI check fails if the three copies (or
their version) drift. Edit this file and re-sync the skills together — never one
in isolation. Nothing platform-specific (tool names, hook mechanics, plugin
paths) belongs in this text; that lives in each skill outside the managed core.

## Two-Layer Governance

```
ctl task (parent)  — declared scope, gates, boundaries (the ctl ledger)
  └─ host subtasks — your runtime's own step/todo tracking, always inside the
                     parent task's write_allow
```

You **proactively** create the parent ctl task before risk-bearing work, then
break it into subtasks with your host's native mechanism. Enforcement is done by
the host's ctl gate (see the platform section): mutating actions outside scope
are blocked, and if `ctl` is unavailable mutating tools **fail closed** (blocked)
until it responds — you cannot work around a block by retrying; create or widen a
task, or redirect the work.

## When to Engage (proactive) vs. Skip

Create a ctl task when you detect:
- multi-file changes (2+ files to modify);
- a feature / bugfix / refactor with a clear objective;
- a problem that needs investigation **and** code changes;
- any work that benefits from an audit trail and scope enforcement.

Skip control — work freely — for:
- pure conversation / Q&A;
- read-only exploration;
- a trivial single-file edit (typo, comment);
- when the user explicitly says "skip control".

## Active-Task Binding (ambiguity)

A gated tool call is governed by the task that dispatched it. When more than one
task is active, **bind explicitly** to the intended task id (the host forwards it
to `ctl hook gate`). Do not rely on implicit "there is only one active task"
fallback — that is incidental behavior, not a contract.

## Scope & Protected Paths

`read_scope`, `write_allow`, and `write_deny` are explicit. **`write_allow` is
ALWAYS minimal** — start narrow and widen only with explicit approval via
`ctl task revise`. Never write protected paths: `.git/`,
`.ctl/tasks/*/events.jsonl`, `schemas/`, `Cargo.toml`. Never hand-edit
`events.jsonl` or `task.json` (append-only canonical truth; projections are
rebuilt). Never bypass the gate.

Boundary auto-inference (start here, then minimize):

| Signal | write_allow |
|---|---|
| Single-file fix | that file only |
| Module change | the one module directory |
| Cross-module refactor | one entry per module |
| Schema change | schema dir + the owning domain module |
| Test addition | the test dir or matching source dir |

## Task Lifecycle

```
ctl task create --id <id> --objective "<text>" \
  --read-scope <path>... --write-allow <path>... --gates <gate>...
ctl task ready  --id <id>
ctl task start  --id <id>
# ... work within scope ...
ctl task submit --id <id>     # → Review; the commit window opens here
# review the whole task diff, record the verdict (see Audit), commit the
# in-scope work during Review, then:
ctl task finish  --id <id>
ctl task archive --id <id>
```

Canonical order: **submit → record passing audit + commit in-scope work (both in
Review) → finish → archive.** `ctl task finish` is **hard-gated**: it refuses
without (a) a fresh passing completion audit recorded after the last `submit`,
(b) fresh gate evidence bound to the current tree, and (c) a clean working tree.
If finish reports stale evidence, rerun the gates and re-audit:

```
ctl gate run --id <id> --gate <gate>   # for each required gate, then re-audit
```

## Audit & Reviewer Identity

The completion audit is mandatory and hard-gates finish. The **implementer
cannot record its own passing audit** — the recording actor must differ from
whoever started/implemented the task:

```
CTL_ACTOR=<reviewer-id> ctl review accept --id <id> --note "<summary>"   # pass
CTL_ACTOR=<reviewer-id> ctl review reject --id <id> --note "<findings>"  # fail
```

A `reject` may come from anyone (including the implementer self-flagging). A
prior round's pass is **stale** once the task is re-submitted — re-audit after
rework. A verdict is evidence on the ledger, not chat; never hand-write it.

## Review Gates (read-only reviewer)

Two review gates, both via a **read-only** reviewer (always safe to dispatch,
even before a task exists, with no write risk):

- **Edit review** — before an out-of-scope or risk-bearing change, have the
  reviewer assess the proposed diff and honor the verdict.
- **Completion audit** — after `submit`, the reviewer runs the closure checklist
  over the whole diff (build/test/lint **evidence**, not assertions) and emits a
  verdict, which you record (above). This gate is hard.

Before an edit review, check whether the write collides with another active
task's `write_allow`; on overlap, sequence the tasks rather than writing
concurrently to a shared path.

## Brainstorm / Research Routing

- Unclear requirements, a new feature, or scoping a complex change → run the
  **brainstorm** flow to diverge/converge before creating the implementation
  task.
- A question answered by producing **evidence rather than code** → a
  **research/spike** task: it completes by recording evidence + uncertainty
  outcomes, not a diff.

## Honest Disclosure

Telemetry, model/oracle output, and human backfill are **evidence, not state**:
they never relax scope, and an unknown signal fails closed. Record the unknowns a
task carries (uncertainty) and disclose them; never present model judgement as a
verdict. "Done" requires evidence artifacts (build/test/run output), not claims —
"where is the data?"

## Iron Law (diagnosis)

Never suggest fixes before completing the diagnosis. Every finding follows:
**Symptom → Source → Consequence → Remedy.** Severity: Critical / Warning /
Suggestion.

## Anti-Patterns

- Starting multi-file work without creating a ctl task.
- Writing outside `write_allow`, or widening scope to root.
- Hand-editing `events.jsonl` or `task.json`.
- Recording your own passing completion audit (reviewer must ≠ implementer).
- Suggesting a fix without diagnosing the root cause.
- Treating model/evidence output as authoritative state.
<!-- ctl:control-guard-core:end -->

## Claude Code Integration (platform-specific)

`.claude/hooks/ctl-gate.py` (PreToolUse) does the enforcement — you do not
replicate it:

- **PreToolUse** gates the mutating tools `Write` / `Edit` / `MultiEdit` / `Bash`
  via `ctl hook gate`; it returns a **deny** decision (blocking the tool) on an
  out-of-scope or wrong-phase verdict, and **fails closed** for `Write` / `Edit` /
  `MultiEdit` when `ctl` is unavailable (`Bash` is not, to avoid locking out the
  shell).
- **SessionStart** (`.claude/hooks/ctl-context.py`) injects the active task
  boundaries (scope, phase, task id) at session start.
- The gate reads `CTL_TASK_ID` from the environment to bind a call to its task
  when several are active.

### Subagent dispatch (read-only only)

ctl governs writes; you choose what to dispatch. Claude Code picks a subagent by
its `description`, so route read-only work by phase — but **only read-only work is
dispatched**:

| Phase / work | Role | Governance |
|---|---|---|
| read-only investigation, broad search, codebase Q&A | `Explore` (built-in) | read-only — always safe |
| Claude Code / SDK / API questions | `claude-code-guide` (built-in) | read-only — always safe |
| diagnosis & falsifiable root-cause (`ctl-diagnose`) | `ctl-oracle` (`.claude/agents/`) | read-only — always safe |

**Writes stay inline in the main agent.** Only the main agent reliably carries
`CTL_TASK_ID` and routes `Write` / `Edit` / `Bash` through the gate. Do **not**
dispatch file edits to subagents: a subagent runs in an isolated context, does not
inherit `CTL_TASK_ID`, and whether its tool calls reach the PreToolUse gate at all
is a **host** behavior that is unverified (see `.claude/subagent-dispatch.md`).
Writable subagent roles (a designer/oracle equivalent) are therefore deferred
until that host behavior is confirmed. So: dispatch read-only investigation and
diagnosis; do the implementation inline, inside the active task's `write_allow`.

Workflow phases (see `.agent/protocols/workflow-skills.md`): `ctl-grill-with-spec`
to align from first principles, `ctl-to-prd` to synthesize a PRD, `ctl-to-tasks`
to break it into vertical task proposals, `ctl-tdd-loop` for red→green
implementation, and `ctl-handoff` to compact context for the next agent. Diagnose
a blocked write with `ctl boundary explain --path <path>`.

Spec lifecycle: run `ctl-spec-bootstrap` to introduce ctl to a project (migrate an
existing workflow, then generate `.ctl/spec/`) or to refresh specs after a large
refactor. After `ctl task finish` succeeds and the task revealed a non-obvious
pattern, route to `ctl-spec-update` to capture it into `.ctl/spec/` (writing there
requires the path in the active task's `write_allow`).
