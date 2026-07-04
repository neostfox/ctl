# Logging & Output Guidelines

> CLI output conventions and diagnostic formatting for the `control` binary.

---

## Overview

This project is a synchronous Rust CLI. It does not use a logging framework (no `tracing`, `log`, or `env_logger` crate). All user-facing output goes through `println!`/`eprintln!` directly, structured for both human readability and script consumption.

---

## Output Channels

| Channel | Use | Example |
|---------|-----|---------|
| `stdout` | Normal command output (results, status, lists) | `Task 'fix-bug' created.` |
| `stderr` | Diagnostics, warnings, error messages | `Warning: no schema files found in schemas/` |
| Exit code 0 | Success | |
| Exit code 1 | Recoverable error (validation failure, not found) | |
| Exit code 2 | Usage error (wrong arguments) | clap handles this automatically |

---

## Output Formatting

### Human-Readable (Default)

Each successful command outputs:
1. A one-line confirmation with the task ID.
2. A suggestion for the next command.

```
Task 'fix-bug' created (Planning phase).
Next: control task ready --id fix-bug
```

### Machine-Readable (`--json` flag, future)

Status commands support `--json` for script consumption. Output is pretty-printed JSON:

```json
{
  "id": "fix-bug",
  "phase": "in_progress",
  "is_held": false,
  "objective": "Fix config parsing",
  "gates": ["cargo_check", "cargo_test"]
}
```

> Note: `--json` flag is planned but not yet implemented in M0.

---

## Output Patterns by Command

### Task Lifecycle Commands

```
control task create → "Task '<id>' created (Planning phase).\nNext: control task ready --id <id>"
control task ready  → "Task '<id>' marked ready.\nNext: control task start --id <id>"
control task start  → "Task '<id>' started.\nPhase: InProgress"
control task status → Multi-line: phase, objective, scope, gates, hold status
```

### Validation Commands

```
control validate           → "All event logs valid." or list of issues
control schema validate    → "Schema valid: <file>" or validation errors
control boundary check     → "Path '<path>' is within scope." or violation details
control architecture check → "Architecture check passed." or list of violations
```

### Rebuild Commands

```
control replay → "Replayed task '<id>'. Projection updated."
control doctor → Multi-line diagnostic report
```

---

## Error Output Format

Errors go to `stderr` with the format:

```
Error: <what went wrong>

<optional detail>

<optional suggestion>
```

Examples:

```
Error: Task 'fix-bug' not found.
Run 'control init' first if this is a new workspace.
```

```
Error: Cannot start task 'fix-bug': task is in Planning phase.
Run 'control task ready --id fix-bug' first.
```

---

## Diagnostic Output (`control doctor`)

The `doctor` command checks workspace health and reports:

1. `.ctl/tasks/` existence and structure
2. Schema files presence
3. Event log integrity (seq ordering, command_id uniqueness)
4. Projection consistency (replay matches stored `task.json`)

Each check outputs a line:

```
[OK] .ctl/tasks/ exists
[OK] schemas/ found (4 schemas)
[WARN] No context snapshot for task 'fix-bug'
[FAIL] Event seq gap in task 'other-task': expected 3, got 5
```

---

## Common Mistakes

### Mistake 1: Debug Output in Production Path

```rust
// BAD: debug output leaks to user
println!("state = {:?}", state);

// GOOD: only in --verbose or --debug flag
if verbose { eprintln!("[DEBUG] state = {:?}", state); }
```

### Mistake 2: Unstructured Error Printing

```rust
// BAD: raw error chain
eprintln!("{:?}", err);

// GOOD: human-friendly message
eprintln!("Error: {}", err);
```

### Mistake 3: Missing Next-Step Hint

Every mutating command should suggest the next logical command. Users should never have to guess what to do next.

---

## Exit Code Contract

| Code | Meaning |
|------|---------|
| 0 | Command succeeded |
| 1 | Business logic error (task not found, validation failed, illegal transition) |
| 2 | CLI usage error (handled by clap) |

Non-zero exit codes MUST include an error message on stderr explaining what happened and how to resolve it.
