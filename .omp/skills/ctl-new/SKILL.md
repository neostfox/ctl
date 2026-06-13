---
name: ctl-new
description: "Create and start a control-plane task with OMP worktree isolation. Accepts natural language or pre-approved boundary fields. Handles all ctl commands internally."

# /ctl-new — Create Task + Start OMP Run

This skill handles ALL `ctl` commands for task creation. The main agent never calls `ctl` directly — it invokes this skill.

## Input

Either:
- **Natural language**: "/ctl-new 修复 auth 超时逻辑" — you infer everything, propose, then execute after approval.
- **Pre-approved fields** (from control-guard proposal): id, objective, read_scope, write_allow, gates — skip proposal, execute immediately.

## Step 1: Infer or receive boundaries

If called with natural language, follow the control-guard inference rules:
1. Read the codebase to locate relevant files.
2. Infer id, objective, read_scope, write_allow, gates.
3. Present proposal. Wait for approval.

If called with pre-approved fields (control-guard already got approval), skip to Step 2.

## Step 2: Execute task creation

Run these commands in sequence from the project root:

```powershell
ctl task create `
  --id <id> `
  --objective "<objective>" `
  --read-scope <path1> --read-scope <path2> `
  --write-allow <path1> --write-allow <path2> `
  --gates cargo_fmt_check --gates cargo_check --gates cargo_test
```

If `cargo_fmt_check` is not needed or `cargo_clippy` is requested, adjust the `--gates` accordingly.

## Step 3: Transition lifecycle

```powershell
ctl task ready --id <id>
ctl task start --id <id>
```

## Step 4: Start OMP worktree run

```powershell
ctl run start --id <id> --adapter omp
```

## Step 5: Verify

```powershell
ctl task status --id <id>
```

Confirm: Phase is `InProgress`, active run exists, lease is active.

## Output

```
✅ <id> → in_progress (OMP worktree active)

  Worktree: .ctl/tasks/<id>/worktree/
  Gates: cargo_fmt_check, cargo_check, cargo_test
  
  Implement inside the worktree.
  When done → /ctl-apply <id>
```

## Error Handling

- **Path rejected**: Run `ctl boundary explain --path <path>`, fix path, retry. Never widen to root.
- **Task id exists**: Propose new id. Never mutate existing events.
- **Active run blocks start**: Suggest `/ctl-abort <id>` first.
- **Any command fails**: STOP, report error, do not skip steps.

## Rules

- `write_allow` is always minimal — specific files preferred over directories.
- All commands use `ctl` directly.
