---
name: ctl-apply
description: "Diff and apply worktree changes back to main workspace. Can be invoked by the user or auto-chained by control-guard after implementation completes."
---

# /ctl-apply — Diff + Apply Worktree Changes

This skill handles ALL `ctl` commands for workspace diff and apply. The main agent never calls these commands directly.

## Input

- **task-id**: The task identifier (required)

## Step 1: Show diff

```powershell
ctl workspace diff --id <task-id>
```

Present to the user:
- `files_modified`: changed files
- `files_added`: new files
- `files_deleted`: removed files
- `high_risk`: flagged files

## Step 2: Safety checks

**Scope check**: Every file must be within `write_allow`. If any is out of scope → STOP. Do not apply.

**High-risk check**: If changes include deletions, dependency changes, public API changes, Git operations, or security policy changes → request approval first:

```powershell
ctl approval request --id <task-id> --reason "<what and why>" --ttl 86400
```

Wait for user to grant or deny:
```powershell
ctl approval grant --id <task-id> --request <request-id>
# or
ctl approval deny --id <task-id> --request <request-id>
```

**No high-risk changes**: Proceed directly to Step 3.

## Step 3: Apply

```powershell
ctl workspace apply --id <task-id>
```

## Step 4: Verify + prepare for close

```powershell
ctl task status --id <task-id>
```

Create `agent-output.json` for the ingest step:

```json
{
  "source": "omp",
  "touched_files": ["<actually modified files from diff>"],
  "summary": "<what was done>",
  "status": "success"
}
```

Write to `.ctl/tasks/<task-id>/agent-output.json`.

## Output

```
✅ Changes applied for '<task-id>'.
   Modified: <file list>
   
   Auto-routing → /ctl-close <task-id>
```

After success, auto-invoke `/ctl-close <task-id>`.

## Error Handling

- **Lease expired / max use exhausted**: `/ctl-abort <id> "<reason>"`, then restart run.
- **Out-of-scope files**: Do NOT apply. Record in task notes as M5 data.
- **Approval denied**: Do NOT apply. Ask user for direction.
