# Domain Layer Spec

> Conventions for `src/domain/` — the pure, side-effect-free core.

---

## Overview

The domain layer contains the **reducer** (`apply`), **state types** (`TaskState`, `Phase`), and **event definition** (`Event`). It is the single source of truth for state machine logic and must remain free of all side effects.

**Hard constraint** (MODULE-001, MODULE-002): No imports from `cli/`, `infrastructure/`, `adapters/`, `std::fs`, `std::net`, `std::process`, or `std::time`.

---

## Module Structure

```
src/domain/
  mod.rs       → pub mod event; pub mod task; #[cfg(test)] pub mod audit_matrix;
  event.rs     → Event struct + is_valid()
  task.rs      → TaskState, Phase, GateResult, apply() reducer
  audit_matrix.rs → Fixed audit matrix with PASS/ASK/STOP/UNVERIFIED verdicts (test-only)
```

---

## Event Envelope

```rust
pub struct Event {
    pub schema: String,       // Must be "control.event-envelope.v1"
    pub event_id: String,     // UUID v4
    pub command_id: String,   // Idempotency key
    pub task_id: String,      // Task aggregate ID
    pub seq: i64,             // Strictly ascending per task
    pub occurred_at: String,  // ISO 8601
    pub actor: String,        // "human" | "system" | adapter ID
    pub event_type: String,   // Event kind (e.g., "task_created")
    pub payload: Value,       // Type-specific payload
}
```

### Event Validation

```rust
impl Event {
    pub fn is_valid(&self) -> bool {
        self.schema == "control.event-envelope.v1" && self.seq > 0
    }
}
```

Full schema validation happens at the `application` layer via `SchemaValidator`. The domain layer only checks structural validity.

---

## State Machine

```
Planning → Ready → InProgress → Review → Completed
    |         |          |          |
    +---------+----------+----------+→ Cancelled
```

### Phase Enum

```rust
pub enum Phase {
    Planning,
    Ready,
    InProgress,
    Review,
    Completed,
    Cancelled,
}
```

### Orthogonal States

- `is_held: bool` — Hold is separate from phase. Violation, gate failure, or human pause sets hold.
- `is_archived: bool` — Archive is a storage attribute. Only terminal phases (`Completed`, `Cancelled`) can be archived.

---

## TaskState

```rust
pub struct TaskState {
    pub id: String,
    pub phase: Phase,
    pub is_held: bool,
    pub is_archived: bool,
    pub objective: Option<String>,
    pub read_scope: BTreeSet<String>,    // Normalized paths
    pub write_allow: BTreeSet<String>,   // Normalized paths
    pub write_deny: BTreeSet<String>,    // Normalized paths
    pub risk_triggers: BTreeSet<String>,
    pub gates: BTreeSet<String>,         // Canonical gate IDs
    pub gate_results: HashMap<String, GateResult>,
    pub history: Vec<String>,            // Applied event types
    pub last_seq: i64,
    pub processed_commands: HashSet<String>,  // Dedup by command_id
}
```

### Boundary Fields

- Stored as `BTreeSet<String>` for deterministic ordering and dedup.
- Set on `task_created` and fully replaced on `task_revised`.
- `objective`, `read_scope`, `write_allow`, `gates` must be non-empty after creation.
- `scope` (legacy) is rejected by the reducer.

### Gate Results

- Latest result per gate_id (older results are overwritten).
- `GateResult { gate_id, passed, evidence, checked_at }`.
- Completion interlock requires all gates in `self.gates` to have `passed: true`.

---

## Reducer (`apply`)

```rust
pub fn apply(state: &mut TaskState, event: &Event) -> Result<(), String>
```

### Rules

1. **Command_id dedup**: If `command_id` was already processed, skip (idempotent).
2. **Seq ordering**: `event.seq` must be `> state.last_seq`.
3. **Phase transition**: Each event type has legal phase prerequisites.
4. **Boundary extraction**: `task_created` and `task_revised` extract boundary fields from payload.
5. **Hold guard**: Hold blocks `start`, `submit`, `finish`.
6. **Legacy rejection**: Payloads containing `"scope"` key are rejected.

### Event Type → Transition Table

| Event Type | From Phase | Effect |
|------------|-----------|--------|
| `task_created` | (new state) | Set objective, scope, gates; phase = Planning |
| `task_revised` | Planning | Replace boundary fields |
| `task_marked_ready` | Planning | Phase = Ready |
| `task_started` | Ready | Phase = InProgress |
| `task_submitted_for_review` | InProgress | Phase = Review |
| `task_reopened` | Review | Phase = InProgress |
| `task_completed` | Review | Phase = Completed (gate check in application layer) |
| `task_cancelled` | Any non-terminal | Phase = Cancelled |
| `task_archived` | Completed/Cancelled | is_archived = true |
| `gate_checked` | Any | Update gate_results |
| `hold_entered` | Any | is_held = true |
| `hold_exited` | Any (held) | is_held = false |
| `boundary_violation_recorded` | Any | is_held = true + record violation |

---

## Testing

- **Fixtures**: `fixtures/reducer_test.jsonl`, `reducer_lifecycle.jsonl`, `reducer_hold.jsonl`, `reducer_revise.jsonl` provide canonical event sequences.
- **Audit matrix**: `domain/audit_matrix.rs` covers schema counter-examples, illegal transitions, path escape, protected files, dependency whitelist, and baseline regression.
- **Exhaustive matching**: New event types must have a branch in `apply`'s match. Missing branches are bugs.

---

## Common Mistakes

### Mistake 1: Accessing Current Time in Reducer

```rust
// BAD: makes replay non-deterministic
let now = chrono::Utc::now();

// GOOD: timestamps come from the event envelope (set by application layer)
```

### Mistake 2: Filesystem Paths in Domain

```rust
// BAD: domain should not know about path separators
let path = std::path::PathBuf::from(&scope_entry);

// GOOD: domain stores normalized strings; infrastructure handles path logic
state.read_scope.insert(scope_entry.to_string());
```

### Mistake 3: Missing Command_id Dedup

Without dedup, replaying the same event stream twice would double-apply events. Always check `processed_commands` before mutating state.
