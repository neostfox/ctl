# Research: subagent dispatch under ctl governance (`.claude`)

> Task: `claude-subagent-dispatch` (research). Question: why does Claude Code do
> code/doc **writing** in the main agent rather than dispatching subagents, and
> what dispatch / control-guard routing should `.claude` adopt?
>
> This is a research finding, **not a verdict**. Basis is separated below;
> the one blocking unknown is called out as open uncertainty, not hidden.

## TL;DR

Writing stays in the main agent for two compounding reasons: (1) `.claude` defines
**no subagent roles** to route to, and (2) under ctl, a write is bound to its task
via the `CTL_TASK_ID` env var, which **subagents do not inherit** — so a dispatched
subagent cannot reliably carry the active-task binding, and it is undocumented
whether its tool calls even reach the PreToolUse gate. Inline (main-agent) writes
are therefore the safe default. **Recommendation:** dispatch read-only
*research/exploration* to subagents now (safe); keep **writes inline** by default
(mirror Codex `dispatch_mode: inline`). The blocking uncertainty (U-1) is now
**resolved against write-dispatch**: Claude Code's PreToolUse cannot gate the
`Task`/subagent tool, so writable `.claude` subagents stay deferred **by design**,
not pending a sandbox probe (see *Resolved — U-1* below).

## ObservedBasis (what was read in this repo)

- `.claude/` contains only governance shims — `hooks/ctl-gate.py`,
  `hooks/ctl-context.py`, `settings.json`, `settings.local.json`. There are **no
  agent/role definitions** (no `.claude/agents/`).
- `.claude/settings.json` registers `ctl-gate.py` as a `PreToolUse` hook for
  matcher `Write|Edit|MultiEdit|Bash`.
- `.claude/hooks/ctl-gate.py` calls `ctl hook gate`, and forwards the dispatch
  binding by reading **`CTL_TASK_ID` from `os.environ`** → `--task <id>`
  (lines 54–56). With no `CTL_TASK_ID`, the gate falls back to the most-recently
  modified `in_progress` task; under multiple active tasks that is ambiguous and
  fails closed (`multiple_active`).
- By contrast, `.opencode/` defines real subagent roles — `agent/designer.md`,
  `agent/oracle.md` (writable roles that "require an active `in_progress` task")
  — plus a control-guard dispatch table and a `task`-tool gate in
  `plugins/ctl-gate.ts`. `.omp/` has the analogous role set. Only `.claude` lacks
  them.
- `.ctl/config.yaml` already records the same trade-off for Codex:
  `dispatch_mode: inline` (default) because "Codex sub-agents run with
  `fork_turns=none` isolation and can't inherit the parent session's task
  context."

## ConfirmedBasis (Claude Code mechanics, from official docs)

Verified via the `claude-code-guide` agent against code.claude.com/docs:

- **Subagents run in an isolated, fresh context** with their own system prompt and
  permissions; the parent sees only the final result. They do **not inherit the
  main agent's environment variables**.
- **Dispatch is description-driven and automatic**: Claude routes a task to a
  subagent when it matches that subagent's `description`. So dispatch targets only
  exist if subagents are defined — and `.claude` defines none.
- **Docs guidance**: delegate *research / isolated exploration* (read-only) to
  subagents; keep *coordinated multi-file edits* in the main agent. Inline writing
  already matches this guidance.

## Resolved — U-1 (2026-06-20): PreToolUse cannot gate subagent dispatch

The previously-blocking unknown was settled by a read-only `claude-code-guide`
spike during the 0.0.5 orchestration-trust audit. Recorded as uncertainty `U-1`
**RESOLVED** on task `orchestration-trust-audit-v1` (external-authority evidence
`E-1`); see that report's Addendum (`brainstorms/orchestration-trust-audit-v1.md`).

> **Do a subagent's Write/Edit/Bash tool calls trigger the `.claude` PreToolUse
> hook (and does PreToolUse fire for the Task/subagent-spawn tool)? — CONFIRMED
> NO.** Per the official Claude Code hooks docs:
>
> - PreToolUse does **not** match the `Task` / `Agent` / `Skill` tools — adding
>   `Task` to the `settings.json` matcher is an **inert no-op**. (Agent lifecycle
>   has a separate `SubagentStart` event whose deny capability is undocumented.)
> - A spawned subagent's own `Write`/`Edit`/`Bash` calls run in an **isolated
>   context** and do **not** trigger the parent session's PreToolUse hooks; the
>   subagent uses its own frontmatter hooks, and `CTL_TASK_ID` propagation into a
>   subagent's hook environment is unspecified (assume not).

**Consequence — a platform boundary, not a TODO.** Claude↔OpenCode subagent-gating
parity is **structurally impossible via PreToolUse**: OpenCode's session-level
plugin can gate the `task` tool; Claude's PreToolUse model cannot. This *validates*
the design above — subagent writes were never reachable by the gate, so keeping
**writes inline in the main agent is the correct mitigation, not a stopgap**.
Writable `.claude` subagent roles therefore stay deferred **by design**. (The
`ctl-gate.py` regression tests pin the corollary: the hook never consults `ctl`
for a `Task` tool call — `test_ctl_gate.py::test_task_tool_is_ungoverned_and_never_reaches_ctl`.)

## OpenUncertainty (still open — must not be hidden)

- **Can subagents spawn nested subagents, and does the `SubagentStart` event
  expose a deny capability?** Undocumented.
- A live in-repo probe of the (now-resolved) gating question was **declined by a
  built-in subagent** on safety grounds — built-in subagents apply independent
  judgment and may refuse governance-sensitive writes, a further reason
  write-dispatch is not "free". Any future writable-role experiment belongs in a
  **throwaway sandbox checkout**, not here.

## Why writing stays in the main agent (answer)

1. **No routing targets.** Description-driven dispatch has nothing to match —
   `.claude` ships zero subagent roles.
2. **Binding can't follow a dispatched write.** ctl binds a write to a task via
   `CTL_TASK_ID`; subagents don't inherit env, so a dispatched write loses its
   task binding (works only by luck with exactly one active task).
3. **Gate reachability for subagents is unverified.** If the PreToolUse hook does
   not fire for subagent calls, dispatched writes are ungoverned.
4. **Precedent + docs agree.** Codex is already pinned to `inline` for the same
   isolation reason, and Claude Code docs recommend inline for coordinated edits.

## Recommendation for `.claude`

1. **Adopt read-only dispatch now (safe).** Add a description-driven routing note
   to the ctl-managed `CLAUDE.md` block: send *investigation / research /
   broad-search* to read-only subagents (built-in `Explore`, `claude-code-guide`).
   No write → no binding/hook concern. (This research itself used that pattern.)
2. **Keep writes inline by default.** Make the `.claude` analog of Codex
   `dispatch_mode: inline` explicit: the main agent holds `CTL_TASK_ID` and its
   Write/Edit/Bash calls reliably reach the gate. Document it so it is a chosen
   policy, not an accident.
3. **Gate write-dispatch on resolving the unknown.** Before defining any writable
   `.claude` subagent (e.g. a designer/oracle mirror of `.opencode/agent/`):
   a. In a **sandbox checkout**, verify whether subagent Write/Edit/Bash calls
      fire the PreToolUse hook.
   b. If yes, give the subagent its binding explicitly — instruct it to pass
      `--task <id>` (or set `CTL_TASK_ID`) on bash-mediated ctl calls; note that
      tool-level `Write`/`Edit` can't carry an env var, so writable subagents may
      need to route edits through bash or the harness must propagate
      `CTL_TASK_ID`.
    c. Each writable role must require an active `in_progress` task and never
       relax a boundary (mirror the `.opencode` designer/oracle contract).
4. **Do not** enable autonomous write-dispatch in `.claude` until 3a is settled.

## Suggested follow-up tasks (not done here)

- `sandbox-verify-subagent-gate` — empirically test 3a in a throwaway checkout.
- `claude-readonly-dispatch-routing` — add the read-only routing note to the
  ctl-managed `CLAUDE.md` block (recommendation #1), a small, safe `.claude` edit.
