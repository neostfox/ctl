---
name: ctl-close
description: "Close a task end-to-end: ingest evidence, run gates, submit, finish, archive. Auto-invoked by control-guard after /ctl-apply succeeds."
---

# /ctl-close — Ingest + Gates + Submit + Finish + Archive

This skill handles ALL `ctl` commands for task closure. The main agent never calls these commands directly.

## Input

- **task-id**: The task identifier (required)

## Prerequisites

- Task is `InProgress`
- `workspace apply` was run
- `.ctl/tasks/<task-id>/agent-output.json` exists

If any prerequisite fails, STOP and tell the user what's missing.

## Step 1: Ingest evidence

```powershell
ctl run ingest --id <task-id> --adapter omp --result .ctl/tasks/<task-id>/agent-output.json
```

If "out of write scope": fix `touched_files` in agent-output.json, retry. Do not skip.
If "no active run": check status — may already be ingested or aborted.

## Step 2: Run gates

Run ALL gates listed in the task. Check task status for gate list:

```powershell
ctl gate run --id <task-id> --gate cargo_fmt_check
ctl gate run --id <task-id> --gate cargo_check
ctl gate run --id <task-id> --gate cargo_test
```

Add `cargo_clippy` if the task specified it.

**Gate fails**: Fix the source issue, re-run only the failed gate. Do NOT proceed until ALL gates pass.

Show results:
```
Gates:
  ✓ cargo_fmt_check: PASS
  ✓ cargo_check: PASS
  ✓ cargo_test: PASS
```

## Step 3: Submit

```powershell
ctl task submit --id <task-id>
```

## Step 4: Finish (completion interlock)

```powershell
ctl task finish --id <task-id>
```

If fails: run `ctl audit --id <task-id>` to see what blocks. Fix, retry.

## Step 5: Archive

```powershell
ctl task archive --id <task-id>
```

## Step 6: Final status

```powershell
ctl task status --id <task-id>
```

```
✅ <task-id> → completed & archived.
   All gates PASS. No active run or lease.
```

## Error Handling

- **Finish interlock error**: `ctl audit --id <task-id>`, fix the blocker, retry finish.
- **Gate keeps failing**: Fix root cause in source, re-run the gate.
- **Task held**: Cannot submit/finish while held. Check status, resolve hold.
