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
  gates/mod.rs         → GateTemplate registry + gate runner
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

1. **Define** the built-in gate templates (`GATE_TEMPLATES`) and resolve the merged set (built-in ∪ project `[[gate]]`).
2. **Resolve** a gate id against the merged set via `resolve_gate` (for validation).
3. **Execute** gates under EXEC-002 supervision (the runner is **live** at V1 — not an M0 stub).

### Gate Template Registry

`GATE_TEMPLATES` is the **built-in** list — the `cargo_*` gates that ship with ctl. It is **extended at load** by `[[gate]]` tables in `.ctl/config.toml`, which declare project gates in the same fixed `{command, args}` shape (never a shell). The merged set is what validation and execution resolve against:

- built-in ids (`cargo_fmt_check`, `cargo_check`, `cargo_test`, `cargo_clippy`) are **reserved** — a project `[[gate]]` whose `id` collides with one is rejected at load (fail-closed, so a broken config never silently relaxes the gate set);
- `EXEC-001` holds on both sides: every gate runs as a fixed `command + args` array, never an arbitrary shell string;
- the EXEC-002 runner is **live** (ctl is at V1) — gates actually execute under supervision.

```rust
pub static GATE_TEMPLATES: &[GateTemplate] = &[
    GateTemplate { id: "cargo_fmt_check", command: "cargo", args: &["fmt", "--check"] },
    GateTemplate { id: "cargo_check",     command: "cargo", args: &["check"] },
    GateTemplate { id: "cargo_test",      command: "cargo", args: &["test"] },
    GateTemplate { id: "cargo_clippy",    command: "cargo", args: &["clippy", "--", "-D", "warnings"] },
    // ... plus any [[gate]] entries loaded from .ctl/config.toml
];

pub fn run_gate(gate_id: &str, working_dir: &Path) -> Result<GateRunResult> {
    let template = resolve_gate(gate_id, working_dir)
        .ok_or_else(|| anyhow!("Unknown gate '{}' — must be a built-in template or a [[gate]] entry in .ctl/config.toml", gate_id))?;
    // ... execute command + args under EXEC-002 supervision
}
```

`resolve_gate` checks the built-in `GATE_TEMPLATES` first, then the project `[[gate]]` set loaded from `.ctl/config.toml`; if neither matches, the id is rejected — at validation time for task definitions, and again at execution time. This enforces EXEC-001 on both paths.

---

## Common Mistakes

### Mistake 1: Bypassing FileEventStore for Writes

Never write `events.jsonl` directly. All writes go through `store.append()`.

### Mistake 2: Forgetting Protected Path Check

New protected paths must be added to `PathNormalizer::new()` AND documented in `ARCHITECTURE_GUARDRAILS.md`.

### Mistake 3: Using Unresolvable Gate IDs

Every gate ID must resolve to a built-in `GATE_TEMPLATES` entry OR a project `[[gate]]` in `.ctl/config.toml`. Unresolved IDs are rejected at validation/load time — the validation message names both sources. (Reserve the `src/` `GATE_TEMPLATES` edit for gates that ship with ctl; project gates go in `.ctl/config.toml`.)
