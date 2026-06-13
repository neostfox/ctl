---
name: ctl-abort
description: "Abort an active OMP run, revoke lease, clean worktree. Bayesian diagnosis is auto-triggered by control-guard before this skill is invoked."
---

# /ctl-abort — Abort OMP Run

This skill handles ALL `ctl` commands for run abort. The main agent never calls these directly.

By the time this skill is invoked, control-guard has already applied Bayesian reasoning to diagnose the failure. Use the diagnosis to write a specific abort reason.

## Input

- **task-id**: The task identifier (required)
- **reason**: Why the run is being aborted (required, specific sentence from Bayesian diagnosis)

## Step 1: Check state

```powershell
ctl task status --id <task-id>
```

Confirm there is an active run. If not, abort is not needed.

## Step 2: Abort

```powershell
ctl run abort --id <task-id> --reason "<reason>"
```

## Step 3: Verify cleanup

```powershell
ctl task status --id <task-id>
```

Confirm: no active run, no active lease, worktree removed.

```
✅ Run aborted for '<task-id>'.
   Reason: <reason>
   Worktree cleaned. Lease revoked.
```

## When to abort vs cancel

- **Abort**: Run is broken but the task is still valid. Retry with new run.
- **Cancel**: Whole task is abandoned → `ctl task cancel --id <task-id>`.

## Rules

- Always provide a specific reason derived from diagnosis. Not just "error" or "retry".
- Do NOT manually delete worktree dirs or edit events.jsonl. Use this skill.
