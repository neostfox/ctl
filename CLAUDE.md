<!-- ctl:managed:start -->
## ctl workflow

This repository uses `ctl` as its canonical task and evidence control plane.

Before modifying files:

1. Read `AGENTS.md` when present and the relevant documents under `.ctl/spec/`.
2. Inspect the current ctl task and write scope.
3. If no suitable task exists:
   - use brainstorm for ambiguous or multi-option work;
   - create a scoped ctl task;
   - move it through ready/start before writing.
4. Only modify paths allowed by the active task.
5. Protected-path changes must use ctl apply/approval.
6. Do not manually edit canonical ctl event ledgers or bypass enforcement hooks.

During work:

- Use OMP todos for temporary implementation steps.
- Use ctl tasks for durable scope, gates and lifecycle.
- Record brainstorm, research, uncertainty and evidence provenance when applicable.
- Treat model and critic claims as advisory unless supported by an appropriate oracle.

Subagent dispatch (read-only by default):

- Dispatch **read-only** work — investigation, broad search, research, codebase
  questions — to read-only subagents (built-in `Explore`; `claude-code-guide` for
  Claude Code / SDK / API questions). They preserve main-agent context and cannot
  break scope because they never write.
- Keep **writes inline** in the main agent. Only the main agent reliably carries
  the active task's `CTL_TASK_ID` binding and routes its Write/Edit/Bash through
  the ctl gate. Do **not** dispatch file edits to subagents: a subagent runs in an
  isolated context, does not inherit `CTL_TASK_ID`, and it is unverified whether
  its tool calls reach the PreToolUse gate at all.

Before completion:

1. Commit the final changes.
2. Run all required gates against the committed tree.
3. Submit for review.
4. Record a fresh completion audit.
5. Run `ctl task finish`.
6. Do not modify the repository after finish without opening or revising a task.

Canonical control state lives under `.ctl/`.
Legacy workflow directories such as `.trellis/` are not canonical ctl state.
<!-- ctl:managed:end -->
