---
name: control-guard
description: "Control plane entry point (opencode). Proactively routes ctl task lifecycle — scope, gates, audit, finish — while the .opencode/plugins/ctl-gate.ts plugin enforces boundaries and injects context. Findings follow the Iron Law: Symptom → Source → Consequence → Remedy."
---

# Control Guard (opencode)

The **managed core** below is the platform-neutral control-guard protocol,
byte-checked by CI against `.agent/protocols/control-guard.md` and the OMP skill.
Do not edit it here in isolation. opencode-specific mechanics live in "opencode
Integration" after the core.

<!-- ctl:control-guard-core:start version=4 -->
# Control Guard — Core Protocol

CONTROL_GUARD_PROTOCOL_VERSION = 4

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
break it into subtasks with your host's native mechanism. The host's ctl gate
(see the platform section) runs in **observe mode**: a mutating action outside
scope, or with no active task, is **allowed but recorded** to the non-canonical
decision log (`.ctl/decisions.jsonl`) with a model-visible warning. A warning is
a prompt to create or widen a task before continuing — never permission to keep
working ungoverned. The **hard core still denies**: protected paths, dependency
changes without a step-up approval, held tasks, and cross-task write overlap.
If `ctl` is unavailable, path-scoped write tools **fail closed** (blocked) until
it responds.

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
  verdict, which you record (above). This gate is hard. The checklist includes
  the **observation log**: review `ctl decisions` for the task's window —
  ungoverned writes recorded by observe mode are part of the change story;
  explain them or absorb them into scope before accepting.

Before an edit review, check whether the write collides with another active
task's `write_allow`; on overlap, sequence the tasks rather than writing
concurrently to a shared path.

## Pipeline Routing (proposal-first)

The governed pipeline: **triage (this protocol) → align (grill) → PRD → tasks →
execute (tdd) → wrap-up (finish → spec-update)**. Each station's skill declares
its station contract (upstream artifact → produces → downstream consumer); when
routing, report the current station and its artifact so the human always knows
where the pipeline stands.

- **Trivial** (typo, single-file obvious fix) — skip the pipeline; edit directly
  (the gate records ungoverned writes) or use a quick task.
- **Everything else** — before `ctl task create`, run the align station (grill):
  a first-principles proposal and a micro-decision interview — one question at a
  time, each with a recommended answer; facts come from the repo, direction
  comes from the user. **Do not build until the user confirms.**
- Multiple durable tasks → confirmed alignment goes through **PRD** (the
  pipeline's first hard checkpoint) before **tasks**.
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

## opencode Integration (platform-specific)

`.opencode/plugins/ctl-gate.ts` does the enforcement — you do not replicate it:

- **experimental.chat.system.transform**: injects the active task boundaries
  (scope, phase, task id) into the system prompt every call.
- **tool.execute.before**: gates the mutating tools `write` / `edit` / `patch` /
  `bash` / `task` via `ctl hook gate`. Observe mode: an out-of-scope or
  task-less verdict comes back allowed + recorded to the decision log (the
  plugin does not yet surface the warning text to the model — follow-up).
  Hard-core verdicts (protected path, deps step-up, held, overlap, ungoverned
  writable subagent spawn) still **throw** (blocking the tool), and mutating
  tools **fail closed** when `ctl` is unavailable. Read-only tools are never
  blocked.

### Subagent roles (autonomous dispatch)

ctl **governs** subagent spawns; **you choose** which to dispatch. opencode picks a
subagent by its `description`, so route by phase:

| Phase / skill | Role (opencode-native) | Governance |
|---|---|---|
| read-only investigation; review gates (reviewer ≠ implementer) | `explore` | **read-only — always spawnable** |
| architecture & design, ADR / spec authoring (design, `ctl-architecture-review` follow-up) | `designer` | writable — needs an active in_progress task |
| diagnosis & hard reasoning, falsifiable root-cause (`ctl-diagnose`) | `oracle` | writable — needs an active in_progress task |
| red→green implementation (`ctl-tdd-loop`) | `build` | writable — needs an active in_progress task |

`explore` is the **only** read-only role and is always safe to dispatch. Writable
roles (`build` / `designer` / `oracle`) are **blocked without an active task**;
once allowed they inherit the dispatching task's `write_allow` — bind them with
`CTL_TASK_ID` when several tasks are active. The `task`-tool gate enforces all of
this — you do not replicate it. `explore` and `build` are opencode built-ins;
`designer` and `oracle` are defined in `.opencode/agent/*.md`. The roster mirrors the
`.omp` set (`explore` read-only; `build` / `designer` / `oracle` writable) under
opencode-native names.

Subtasks: use opencode's native task/todo tracking within the parent's
`write_allow`. When several tasks are active, bind one with the `CTL_TASK_ID` env
var. Diagnose a blocked write with `ctl boundary explain --path <path>`. The
plugin contract is covered by `bun test --cwd .opencode`. Workflow phases (see
`.agent/protocols/workflow-skills.md`): `ctl-grill-with-spec` to align from first
principles, `ctl-to-prd` to synthesize a PRD, `ctl-to-tasks` to break it into
vertical task proposals, `ctl-tdd-loop` for red→green implementation, and
`ctl-handoff` to compact context for the next agent.
