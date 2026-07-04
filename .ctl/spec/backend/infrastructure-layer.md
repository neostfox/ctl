# Infrastructure Layer Spec

> Conventions for `src/infrastructure/` — side-effect-containing modules.

---

## Overview

The infrastructure layer implements all I/O: file storage, path normalization, schema validation, and gate execution. It depends on `domain/` types but never the reverse.

```
src/infrastructure/
  mod.rs               → Module declarations
  store/mod.rs         → FileEventStore
  boundary/normalizer.rs → PathNormalizer
  gates/mod.rs         → GateTemplate registry + stub runner
  schema_validator.rs  → JSON Schema validation
```

---

## FileEventStore (`store/`)

### Responsibilities

1. **Append** events to per-task `events.jsonl`.
2. **Read** events back (single task or all tasks).
3. **Write** `task.json` projection (atomic temp-file rename).
4. **Manage** task directory creation and listing.

### Key API

```rust
impl FileEventStore {
    pub fn init(project_root: &Path) -> Result<Self>
    pub fn open(project_root: &Path) -> Result<Self>
    pub fn append(&self, event: &Event) -> Result<()>
    pub fn read_for_task(&self, task_id: &str) -> Result<Vec<Event>>
    pub fn read_all(&self) -> Result<Vec<Event>>
    pub fn write_task_view(&self, task_id: &str, state: &TaskState) -> Result<()>
    pub fn task_ids(&self) -> Result<Vec<String>>
    pub fn task_dir(&self, task_id: &str) -> Result<PathBuf>
    pub fn events_path(&self, task_id: &str) -> Result<PathBuf>
}
```

### Storage Layout

```
.ctl/tasks/<task-id>/
  events.jsonl
  task.json
  context.json    # M2+ baseline snapshot
```

### Conventions

- Append uses `OpenOptions::new().create(true).append(true)`.
- Projection writes use `write(task.json.tmp) → rename(task.json)` for atomicity.
- Task IDs are validated (alphanumeric + `-` + `_`, no leading `.` or `-`).
- Blank lines in JSONL are skipped; malformed JSON returns an error.

---

## PathNormalizer (`boundary/`)

### Responsibilities

1. **Normalize** relative paths (resolve `.`, reject `..`, normalize separators).
2. **Reject** dangerous paths: absolute, UNC, drive prefixes, symlinks, junctions, root escapes.
3. **Enforce** protected path list.

### Key API

```rust
impl PathNormalizer {
    pub fn new(root: PathBuf) -> Self
    pub fn normalize(&self, path_str: &str) -> Result<PathBuf>
    pub fn is_protected(&self, path: &Path) -> bool
    pub fn normalize_batch(&self, field_name: &str, paths: &[String]) -> Result<Vec<String>>
}
```

### Rejection Rules

| Input | Rejection Reason |
|-------|-----------------|
| `/etc/passwd` | Absolute path |
| `../secret` | Parent directory traversal |
| `\\server\share` | UNC path |
| `C:\Windows` | Drive prefix |
| `.git/config` | Protected path |
| `schemas/event.json` | Protected path |
| `Cargo.toml` | Protected path |

### Protected Paths

```rust
vec![".git", ".ctl", ".ctl/tasks", ".control", "schemas", "Cargo.toml", "Cargo.lock"]
```

### Windows Considerations

- Case-insensitive comparison for protected paths on Windows.
- Both `/` and `\` separators must be handled.
- Junctions detected via `fs::symlink_metadata`.

---

## SchemaValidator (`schema_validator.rs`)

### Responsibilities

1. **Load** JSON Schema files from `schemas/` directory.
2. **Validate** JSON instances against named schemas.

### Key API

```rust
impl SchemaValidator {
    pub fn new(schemas_dir: &str) -> Result<Self>
    pub fn validate_instance(&self, instance: &Value, schema_id: &str) -> Result<()>
}
```

### Schema Files

```
schemas/
  control.event-envelope.v1.schema.json
  control.task-definition.v1.schema.json
  control.task-view.v1.schema.json
  control.policy-decision.v1.schema.json
```

### Fallback

If `schemas/` directory doesn't exist, the validator returns `None` and schema validation is skipped gracefully. The application layer handles this:

```rust
if let Some(ref validator) = self.validator {
    validator.validate_instance(&json_val, &event.schema)?;
}
```

---

## Gates (`gates/`)

### Responsibilities

1. **Define** the authoritative list of gate templates.
2. **Lookup** templates by ID for validation.
3. **Execute** gates (M0 stub: returns error; real execution deferred to M2).

### Gate Template Registry

```rust
pub static GATE_TEMPLATES: &[GateTemplate] = &[
    GateTemplate { id: "cargo_fmt_check", command: "cargo", args: &["fmt", "--check"] },
    GateTemplate { id: "cargo_check",     command: "cargo", args: &["check"] },
    GateTemplate { id: "cargo_test",      command: "cargo", args: &["test"] },
    GateTemplate { id: "cargo_clippy",    command: "cargo", args: &["clippy", "--", "-D", "warnings"] },
];
```

### M0 Constraint

`run_gate` is stubbed and always returns an error:

```rust
pub fn run_gate(gate_id: &str, _working_dir: &Path) -> Result<GateRunResult> {
    let _template = find_template(gate_id).ok_or_else(|| anyhow!("Unknown gate template: {}", gate_id))?;
    Err(anyhow!("Gate execution is disabled in M0 until EXEC-002 runner policy is implemented"))
}
```

This enforces EXEC-001: no arbitrary shell execution.

---

## Common Mistakes

### Mistake 1: Bypassing FileEventStore for Writes

Never write `events.jsonl` directly. All writes go through `store.append()`.

### Mistake 2: Forgetting Protected Path Check

New protected paths must be added to `PathNormalizer::new()` AND documented in `ARCHITECTURE_GUARDRAILS.md`.

### Mistake 3: Adding Gate Templates Without Registry

Every gate ID used in task definitions must exist in `GATE_TEMPLATES`. Unknown IDs are rejected at application validation time.
