# Directory Structure

> Backend storage and module layout contracts for this Rust control-layer project.

---

## Scenario: M1 Canonical Task Ledger

### 1. Scope / Trigger

- Trigger: M1 reintroduces task-ledger commands and changes the cross-layer event/store/projection contract.
- Applies to: `src/cli`, `src/application`, `src/domain`, `src/infrastructure/store`, `schemas/`, and `fixtures/`.
- Principle: `events.jsonl` is canonical truth; generated JSON files are projections.

### 2. Signatures

Public M1 CLI surface:

```text
control init
control task create --id <id> --objective <text> \
  --read-scope <path> --write-allow <path> --gates <gate> \
  [--write-deny <path>] [--risk-triggers <trigger>]
control task revise --id <id> [same structured boundary fields]
control task ready --id <id>
control task status --id <id>
control replay [--task <id>]
control validate
control doctor
control schema validate --file <path>
control boundary check --path <path>
control boundary explain --path <path>
control architecture check
```

Application API shape:

```rust
pub struct CreateTaskInput<'a> {
    pub objective: &'a str,
    pub read_scope: &'a [String],
    pub write_allow: &'a [String],
    pub write_deny: &'a [String],
    pub risk_triggers: &'a [String],
    pub gates: &'a [String],
}

pub struct ReviseTaskInput<'a> {
    pub objective: Option<&'a str>,
    pub read_scope: Option<&'a [String]>,
    pub write_allow: Option<&'a [String]>,
    pub write_deny: Option<&'a [String]>,
    pub risk_triggers: Option<&'a [String]>,
    pub gates: Option<&'a [String]>,
}
```

### 3. Contracts

Canonical store:

```text
.trellis/tasks/<task>/events.jsonl  # append-only truth
.trellis/tasks/<task>/task.json     # replay projection
```

Do not write canonical events to `.control/events.jsonl`.

Canonical `task_created` / `task_revised` payload:

```json
{
  "objective": "non-empty string",
  "read_scope": ["path", "..."],
  "write_allow": ["path", "..."],
  "write_deny": [],
  "risk_triggers": [],
  "gates": ["cargo_check"]
}
```

Constraints:

- `objective`, `read_scope`, `write_allow`, `write_deny`, `risk_triggers`, and `gates` are required in canonical task events.
- `objective`, `read_scope`, `write_allow`, and `gates` must be non-empty.
- `scope` is a legacy field and must be rejected in valid canonical task events.
- Path-like boundary fields are normalized before append.
- `TaskState` stores deterministic boundary sets; projection output omits reducer internals such as `processed_commands`.

### 4. Validation & Error Matrix

| Condition | Expected behavior |
|---|---|
| Missing `.trellis/tasks` root | `control init` creates it; `open` commands ask to run init. |
| Empty objective | Reject before append. |
| Empty `read_scope` | Reject before append/schema validation. |
| Empty `write_allow` | Reject before append/schema validation. |
| Empty `gates` | Reject before append/schema validation. |
| Unknown gate id | Reject before append. |
| Protected/canonical event path in write boundary | Reject before append. |
| Legacy `scope` in canonical task payload | Reject by schema/reducer/application checks. |
| Non-M1 CLI command exposed | `control architecture check` fails. |
| `.control/events.jsonl` canonical-store usage reappears | `control architecture check` fails. |

### 5. Good/Base/Bad Cases

- Good: `control task create --id good --objective ok --read-scope src --write-allow src --gates cargo_check` writes `.trellis/tasks/good/events.jsonl` and `.trellis/tasks/good/task.json`.
- Base: `control replay --task good` regenerates the same `task.json` from the same event stream.
- Bad: `control task create --id bad --objective bad --scope src --gates cargo_check` fails because `--scope` is not an M1 flag.
- Bad: a JSON event with payload key `scope` fails schema validation.

### 6. Tests Required

- Unit: reducer rejects legacy `scope` and requires objective/read/write/gates before ready.
- Unit: store writes per-task `.trellis/tasks/<task>/events.jsonl` and projection `task.json`.
- Unit: boundary normalizer rejects canonical task event paths.
- Fixture: reducer lifecycle/hold/revise fixtures use structured boundaries and canonical `cargo_*` gates.
- Architecture: command surface whitelist, schema contract, fixture path/gate checks, no `.control/events.jsonl` canonical store.
- Probe: create → ready → status → replay → validate in a temp workspace.

### 7. Wrong vs Correct

#### Wrong

```json
{
  "objective": "fix bug",
  "scope": ["src/"],
  "gates": ["check"]
}
```

Why wrong:

- `scope` cannot express read vs write permission.
- `check` is not a canonical required gate id.
- Missing `write_deny` and `risk_triggers` breaks the M1 schema contract.

#### Correct

```json
{
  "objective": "fix bug",
  "read_scope": ["src/"],
  "write_allow": ["src/"],
  "write_deny": [],
  "risk_triggers": [],
  "gates": ["cargo_check"]
}
```

---

## Directory Layout

```text
src/
├── cli/              # CLI parsing and architecture checks
├── application/      # command application service; validates before append
├── domain/           # pure reducer and event-derived state
├── infrastructure/   # filesystem store, boundary normalizer, schema validator, gate templates
└── adapters/         # reserved for later milestone adapters

schemas/              # JSON schema contracts
fixtures/             # canonical reducer/schema examples
.trellis/tasks/<id>/  # M1 canonical task ledgers and projections
```
