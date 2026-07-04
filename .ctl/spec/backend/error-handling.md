# Error Handling

> Error handling conventions for this Rust control-layer project.

---

## Overview

This project uses `anyhow::Result<T>` as the universal error type. Domain-layer functions return `Result<_, String>` because the reducer must remain dependency-free. The `application/` layer converts domain errors into `anyhow` errors at the boundary.

---

## Error Types

### Application & Infrastructure: `anyhow::Result<T>`

All application commands and infrastructure operations use `anyhow`:

```rust
pub fn create_task(&self, id: &str, input: CreateTaskInput<'_>) -> anyhow::Result<Event> {
    let existing = self.store.read_for_task(id)?;
    if !existing.is_empty() {
        return Err(anyhow!("Task '{}' already exists", id));
    }
    // ...
}
```

**Conventions**:
- Use `anyhow!()` for domain-validation errors with descriptive messages.
- Use `?` operator for propagated infrastructure/IO errors.
- Never unwrap `anyhow` results silently — always propagate or convert.

### Domain Layer: `Result<_, String>`

The reducer returns `Result<(), String>` because it cannot depend on `anyhow`:

```rust
pub fn apply(state: &mut TaskState, event: &Event) -> Result<(), String> {
    // ...
    Err(format!("Cannot transition from {:?} via {}", state.phase, event.event_type))
}
```

**Why `String`**: The domain layer has zero dependencies beyond `serde`. Using `String` avoids coupling to error frameworks while keeping error messages human-readable.

### Conversion Boundary

The application layer converts domain `String` errors to `anyhow`:

```rust
let state = self.replay_task(task_id)?;
apply(&mut state, &event).map_err(|e| anyhow!(e))?;
```

---

## Error Handling Patterns

### Pattern 1: Validation Before Mutation

Always validate inputs **before** appending events. Events are append-only; a bad event cannot be retracted:

```rust
// GOOD: validate first
validate_task_definition(objective, &read_scope, &write_allow, &gates)?;
let event = self.build_event(id, "task_created", payload)?;
self.validate_and_append(&event)?;
```

### Pattern 2: State Machine Guard

The reducer guards illegal transitions. The application layer catches these and surfaces them:

```rust
// In domain/task.rs apply():
Phase::Ready if event.event_type == "task_started" => { /* ok */ }
Phase::Planning => Err("Cannot start from Planning, must go through Ready".into())
```

### Pattern 3: Compound Validation

For operations with multiple preconditions, collect all issues rather than failing on the first:

```rust
// In validate_store():
let mut issues = Vec::new();
if event.seq <= prev_seq {
    issues.push(format!("Line {}: seq not increasing", line));
}
// ... collect more issues
```

---

## Validation Error Matrix

| Condition | Error Source | Message Pattern |
|-----------|-------------|-----------------|
| Empty objective | `application` | `"Objective must not be empty"` |
| Empty `read_scope`/`write_allow`/`gates` | `application` | `"<field> must not be empty"` |
| Unknown gate ID | `application` | `"Unknown gate template: {id}"` |
| Illegal phase transition | `domain` | `"Cannot {action} from {phase}"` |
| Task already exists | `application` | `"Task '{id}' already exists"` |
| Task not found | `store` | `"Task '{id}' not found"` |
| Duplicate `command_id` | `validate` | `"Duplicate command_id '{id}' at line {n}"` |
| Non-increasing `seq` | `validate` | `"seq {n} not increasing for task {id}"` |
| Path escape (.., absolute, UNC) | `boundary` | `"<type> paths are not allowed: {path}"` |
| Protected path in write scope | `boundary` | `"Path is protected: {path}"` |
| Schema validation failure | `schema_validator` | `"Schema validation: {details}"` |
| Legacy `scope` field | `domain` reducer | Rejects events with `scope` key |

---

## User-Facing Error Messages

All error messages shown to CLI users must:

1. Start with what went wrong (not an internal trace).
2. Include the relevant identifier (task ID, path, field).
3. Suggest the next action when possible.

**Good**: `"Cannot start task 'fix-bug': task is in Planning phase. Run 'control task ready --id fix-bug' first."`

**Bad**: `"called Result::unwrap() on an Err value: ApplyError"`

---

## Common Mistakes

### Mistake 1: Unwrapping in Application Code

```rust
// BAD: panics on invalid state
let state = self.replay_task(task_id).unwrap();

// GOOD: propagates the error
let state = self.replay_task(task_id)?;
```

### Mistake 2: Silently Ignoring Domain Errors

```rust
// BAD: reducer error is swallowed
let _ = apply(&mut state, &event);

// GOOD: surface the error
apply(&mut state, &event).map_err(|e| anyhow!(e))?;
```

### Mistake 3: Using `expect()` Without Context

```rust
// BAD: "precondition failed" tells nothing
let store = FileEventStore::open(root).expect("precondition failed");

// GOOD: descriptive context
let store = FileEventStore::open(root)
    .with_context(|| format!("Failed to open task store at {}", root.display()))?;
```
