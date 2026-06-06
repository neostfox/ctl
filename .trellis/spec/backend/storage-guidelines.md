# Storage Guidelines

> FileEventStore conventions, JSONL format, projection writes, and file layout for the `control` binary.

---

## Overview

This project uses file-based storage with a strict canonical/event-sourced model. There is no database, no ORM, and no migration system. All state is derived from `events.jsonl` append-only logs.

---

## Storage Architecture

```
.trellis/tasks/<task-id>/
  events.jsonl    # Append-only canonical truth (STATE-001)
  task.json       # Replay projection, always rebuildable (STATE-002)
  context.json    # Baseline file hash snapshot (M2+)
```

### Key Invariants

1. **Append-only**: Events are never modified or deleted from `events.jsonl`.
2. **Single writer**: Only `FileEventStore::append` writes events. No direct file access.
3. **Atomic projections**: `task.json` and `context.json` use temp-file + rename.
4. **Per-task isolation**: Each task has its own directory under `.trellis/tasks/`.

---

## FileEventStore API

```rust
// Initialize the store root
FileEventStore::init(project_root) → Result<Self>
// Open existing store
FileEventStore::open(project_root) → Result<Self>
// Append a canonical event
store.append(event: &Event) → Result<()>
// Read all events for a task
store.read_for_task(task_id) → Result<Vec<Event>>
// Read all events across all tasks
store.read_all() → Result<Vec<Event>>
// Write task.json projection (atomic)
store.write_task_view(task_id, &TaskState) → Result<()>
// List all task IDs
store.task_ids() → Result<Vec<String>>
```

---

## JSONL Format

Each line is a self-contained JSON event envelope:

```json
{"schema":"control.event-envelope.v1","event_id":"...","command_id":"...","task_id":"t1","seq":1,"occurred_at":"2026-06-06T10:00:00Z","actor":"human","type":"task_created","payload":{...}}
```

### Rules

- One event per line. No multi-line JSON.
- `seq` must be strictly increasing within a task.
- `command_id` must be globally unique (idempotency key).
- `schema` must be `"control.event-envelope.v1"`.
- Blank lines are skipped during read. Malformed JSON causes an error.

### Write Pattern

```rust
let line = serde_json::to_string(event)?;
let mut file = OpenOptions::new().create(true).append(true).open(path)?;
writeln!(file, "{}", line)?;
file.flush()?;
```

> Future (post-M0): Add `fsync` for crash safety and checksum lines for integrity verification.

---

## Projection Writes

`task.json` is rebuilt from events on every mutation:

```rust
pub fn write_task_view(&self, task_id: &str, state: &TaskState) -> Result<()> {
    let json = serde_json::to_string_pretty(&state)?;
    let task_dir = self.task_dir(task_id)?;
    let temp = task_dir.join("task.json.tmp");
    fs::write(&temp, &json)?;
    fs::rename(&temp, task_dir.join("task.json"))?;
    Ok(())
}
```

### Why Atomic Replace

- Prevents partial reads if a process is interrupted mid-write.
- `rename` on the same filesystem is atomic on both Linux and Windows (NTFS).
- The temp file uses `.tmp` suffix in the same directory to guarantee same-device rename.

---

## Task ID Validation

Task IDs must be filesystem-safe:

```rust
fn validate_task_id(id: &str) -> Result<()> {
    if id.is_empty() { return Err(anyhow!("Task ID must not be empty")); }
    if id.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
        return Err(anyhow!("Task ID contains invalid characters: '{}'", id));
    }
    if id.starts_with('-') || id.starts_with('.') {
        return Err(anyhow!("Task ID must not start with '-' or '.'"));
    }
    Ok(())
}
```

---

## File Layout

```
.trellis/
  tasks/
    fix-bug/
      events.jsonl     # Canonical event log
      task.json        # Replay projection
    add-feature/
      events.jsonl
      task.json
```

### Init Command

`control init` creates `.trellis/tasks/` if it doesn't exist:

```rust
pub fn init(project_root: &Path) -> Result<Self> {
    let tasks_dir = project_root.join(".trellis").join("tasks");
    fs::create_dir_all(&tasks_dir)?;
    Ok(Self { tasks_dir })
}
```

---

## Validation Store

`control validate` reads all event logs and checks:

1. Schema field is `control.event-envelope.v1`
2. `seq` is strictly increasing per task
3. `command_id` is globally unique
4. Each event passes JSON Schema validation (if `schemas/` exists)

---

## Common Mistakes

### Mistake 1: Writing events.jsonl from outside FileEventStore

```rust
// BAD: bypasses validation and seq management
fs::write("events.jsonl", json_line)?;

// GOOD: use the store API
store.append(&event)?;
```

### Mistake 2: Reading task.json Instead of Replaying

```rust
// BAD: stale projection
let state: TaskState = serde_json::from_str(&fs::read_to_string("task.json")?)?;

// GOOD: always replay from events
let state = self.replay_task(task_id)?;
```

### Mistake 3: Non-Atomic Projection Write

```rust
// BAD: interrupted write = corrupt projection
fs::write("task.json", json)?;

// GOOD: temp file + atomic rename
fs::write("task.json.tmp", json)?;
fs::rename("task.json.tmp", "task.json")?;
```

---

## Future (Post-M0)

- File locking for concurrent access (M6+)
- Checksum per line for corruption detection
- `fsync` after append for crash recovery
- Compaction / archival for large event logs
