---
name: control-guard
description: "Control plane entry point. Proactively routes ctl task lifecycle — scope, gates, audit, finish — while the host's ctl gate enforces boundaries and injects context. Findings follow the Iron Law: Symptom → Source → Consequence → Remedy."
---

<!-- integration:omp -->


`.omp/hooks/pre/ctl-context.ts` (OMP native extension) does the enforcement — you
do not replicate it:

- **session_start → context**: injects the active task boundaries into every LLM
  call.
- **tool_call**: gates mutating tools via `ctl hook gate`. Mutating tools here:
  `write`/`edit`, **`bash`**, and the **`task`** (subagent-spawn) tool. Observe
  mode: out-of-scope or task-less writes and commits/pushes outside the Review
  window are allowed but recorded to the decision log; protected paths, deps
  changes without approval, held tasks, and writable subagent spawns without an
  active task are blocked; read-only tools are never blocked.
- **subagent timeout**: blocks `job poll` past the threshold — cancel and
  re-spawn a smaller assignment (configurable via `CTL_SUBAGENT_TIMEOUT_MS`).
- **agent_end / session_shutdown**: spec-drift and unfinished-task reminders.

Subtasks: use OMP **todo** within the parent's `write_allow`. Read-only work —
investigation, codebase research, the review gates, and diagnosis — runs as the
**`scout`** (research) or **`reviewer`** (review gates) subagent; both are always
spawnable with no write risk. The only writable subagent is **`task`**, which
requires an active in_progress task and inherits its boundaries. (OMP has no
`oracle`/`explore` role, and its `designer` is a UI/UX specialist — do not route
architecture or diagnosis there; keep diagnosis read-only via `scout` and act on
its findings inline as the main agent.) Skill routing follows each skill's
trigger contract; the pipeline map lives in the core above (Pipeline Routing).

### Compile gate — LSP + record

The compile gate (`cargo_check`, the floor's only gate) is satisfied via the
HOST's LSP, not by ctl spawning `cargo check`:

1. Run `xd://lsp` diagnostics (rust-analyzer) on the changed Rust files.
2. 0 errors → `ctl gate record --id <task> --gate cargo_check --passed
   --evidence "LSP: 0 errors across <N> files"`.
3. Errors → fix, then re-check; never record a pass while diagnostics fail.

Rationale: ctl stays a control plane (declare + record + enforce); execution
stays in the host. The evidence is host-reported — accepted as the dev-time
compile signal. `ctl gate run --gate cargo_check` remains as a hermetic
fallback. `cargo_test` / `cargo_clippy` / `cargo_fmt_check` are NOT in the
floor — run them via OMP bash when the project needs, and `ctl gate record`
the result if you want it on the ledger.

<!-- integration:opencode -->


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
| architecture & design, ADR / spec authoring (design) | `designer` | writable — needs an active in_progress task |
| diagnosis & hard reasoning, falsifiable root-cause (`ctl-diagnose`) | `oracle` | writable — needs an active in_progress task |
| red→green implementation (`--tdd` interlock) | `build` | writable — needs an active in_progress task |

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
`.agent/protocols/workflow-skills.md`): `ctl-grill-with-spec` to align from
first principles, then `ctl task create` directly (or `ctl prd init` for a
multi-task effort). Opt into red→green with `ctl task create --tdd`;
context compaction is automatic (boundaries inject every call).

### Compile gate — LSP + record

The compile gate (`cargo_check`, the floor's only gate) is satisfied via the
HOST's LSP, not by ctl spawning `cargo check`:

1. Run the host LSP (rust-analyzer) diagnostics on the changed Rust files.
2. 0 errors → `ctl gate record --id <task> --gate cargo_check --passed
   --evidence "LSP: 0 errors across <N> files"`.
3. Errors → fix, then re-check; never record a pass while diagnostics fail.

Rationale: ctl stays a control plane (declare + record + enforce); execution
stays in the host. The evidence is host-reported — accepted as the dev-time
compile signal. `ctl gate run --gate cargo_check` remains as a hermetic
fallback. `cargo_test` / `cargo_clippy` / `cargo_fmt_check` are NOT in the
floor — run them via the host when the project needs, and `ctl gate record`
the result if you want it on the ledger.

<!-- integration:claude -->


`.claude/hooks/ctl-gate.py` (PreToolUse) does the enforcement — you do not
replicate it:

- **PreToolUse** gates the mutating tools `Write` / `Edit` / `MultiEdit` / `Bash`
  via `ctl hook gate`. Observe mode: an out-of-scope or task-less verdict comes
  back allowed + recorded, and the hook forwards its warning to the model as
  `additionalContext` (no permission decision — the normal permission flow is
  untouched). Hard-core verdicts (protected path, deps step-up, held, overlap)
  still return a **deny** decision, and the hook **fails closed** for `Write` /
  `Edit` / `MultiEdit` when `ctl` is unavailable (`Bash` is not, to avoid
  locking out the shell).
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

**Writes stay inline in the main agent by default.** A 2026-07-04 live probe
verified that a subagent's `Write`/`Edit`/`Bash` calls **do pass** the session's
PreToolUse gate (observe-mode warning delivered into the subagent's context,
record in `.ctl/decisions.jsonl`) — superseding the earlier docs-based U-1
"confirmed no" (see `.claude/subagent-dispatch.md`, Addendum). What remains
untested is `CTL_TASK_ID` binding under **multiple active tasks**, so: dispatch
read-only investigation and diagnosis freely; writable dispatch is now
governable but keep coordinated multi-file implementation inline, inside the
active task's `write_allow`, until the binding question is settled.

Workflow phases (see `.agent/protocols/workflow-skills.md`): `ctl-grill-with-spec`
to align from first principles, then `ctl task create` directly (or `ctl prd init`
for a multi-task effort). Opt into red→green with
`ctl task create --tdd`; context compaction is automatic (boundaries inject every call). Diagnose
a blocked write with `ctl boundary explain --path <path>`.

Spec lifecycle: run `ctl-spec` to introduce ctl to a project (bootstrap `.ctl/spec/`
from source) or to refresh specs after a large refactor. After `ctl task finish`
succeeds and the task revealed a non-obvious pattern, route to `ctl-spec` to capture
it into `.ctl/spec/` (writing there requires the path in the active task's `write_allow`).

### Compile gate — LSP + record

The compile gate (`cargo_check`, the floor's only gate) is satisfied via the
HOST's LSP, not by ctl spawning `cargo check`:

1. Run the host LSP (rust-analyzer) diagnostics on the changed Rust files.
2. 0 errors → `ctl gate record --id <task> --gate cargo_check --passed
   --evidence "LSP: 0 errors across <N> files"`.
3. Errors → fix, then re-check; never record a pass while diagnostics fail.

Rationale: ctl stays a control plane (declare + record + enforce); execution
stays in the host. The evidence is host-reported — accepted as the dev-time
compile signal. `ctl gate run --gate cargo_check` remains as a hermetic
fallback. `cargo_test` / `cargo_clippy` / `cargo_fmt_check` are NOT in the
floor — run them via the host when the project needs, and `ctl gate record`
the result if you want it on the ledger.
