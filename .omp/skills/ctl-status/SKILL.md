---
name: ctl-status
description: "Show task or project status. Handles all cargo run -- commands internally."
---

# /ctl-status — Task Status Dashboard

This skill handles ALL `cargo run --` commands for status inspection. The main agent never calls these directly.

## Input

- **task-id** (optional): Specific task. If omitted, show all tasks.

## Single task

```powershell
cargo run -- task status --id <task-id>
```

Highlight: Phase, Gates, Run, Leases, Hold status.

## All tasks

```powershell
cargo run -- report
```

## Audit (detailed)

```powershell
cargo run -- audit --id <task-id>
```

Shows: gate results, evidence counts, violations, interlock verdict.

## JSON output

```powershell
cargo run -- task status --id <task-id> --json
```

For programmatic use.

## Phase guide

| Phase | Meaning | Next action |
|---|---|---|
| `Planning` | Just created | Revise scope, then ready |
| `Ready` | Approved | start |
| `InProgress` | Working | /ctl-apply → /ctl-close |
| `Review` | Submitted | finish or reopen |
| `Completed` | Done | archive |
| `Cancelled` | Abandoned | archive |
