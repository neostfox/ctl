<!-- ctl:managed:start -->
## ctl workflow

This repository uses `ctl` as its canonical task and evidence control plane.

Before modifying files:

1. Read `AGENTS.md` when present and the relevant documents under `.ctl/spec/`.
2. Inspect the current ctl task and write scope.
3. If no suitable task exists:
   - run the alignment station (`ctl-grill-with-spec`) for ambiguous or
     multi-option work — propose with recommendations, micro-confirm with the
     user, then scope;
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
- Keep **writes inline** in the main agent by default. A 2026-07-04 live probe
  verified a subagent's Write/Edit/Bash calls DO pass the PreToolUse gate and are
  observed/recorded like main-agent writes (see `.claude/subagent-dispatch.md`,
  Addendum) — but `CTL_TASK_ID` binding under multiple active tasks is untested,
  so keep coordinated multi-file implementation inline inside the active task's
  `write_allow`; occasional dispatched writes are governable when exactly one
  task is active.

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
