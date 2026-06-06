# Cross-Layer Thinking Guide

> **Purpose**: Think through data flow across layers before implementing.

---

## The Problem

**Most bugs happen at layer boundaries**, not within layers.

In this project, the critical boundaries are:

```
CLI input → Application validation → Event append → JSONL store → Replay → Reducer → Projection → CLI display
```

Each boundary has a specific contract. Breaking it causes:
- Invalid events entering the canonical log
- Stale or inconsistent projections
- Validation bypass at the wrong layer

---

## Layer Boundary Contracts

### CLI → Application

| Concern | Owner | Rule |
|---------|-------|------|
| Argument parsing | `cli/` | clap handles required/optional args |
| Input validation | `application/` | Empty strings, unknown gates, missing fields |
| Path normalization | `application/` (via `PathNormalizer`) | Before boundary checks |
| Error formatting | `cli/` | Human-readable + next-step hint |

### Application → Domain

| Concern | Owner | Rule |
|---------|-------|------|
| Event building | `application/` | `seq`, `command_id`, `occurred_at` |
| Phase transition | `domain/` reducer | `apply()` enforces legal transitions |
| Hold guard | `domain/` reducer | Blocks `start`/`submit`/`finish` when held |
| Legacy field rejection | `domain/` reducer | Rejects `scope` key in payload |

### Application → Store

| Concern | Owner | Rule |
|---------|-------|------|
| Event append | `store/` | One line per event, append-only |
| Projection write | `store/` | Atomic temp-file + rename |
| Task ID validation | `store/` | Alphanumeric + `-` + `_` only |
| Read integrity | `store/` | Skip blank lines, error on malformed JSON |

### Domain Types ↔ JSON Schema

| Concern | Owner | Rule |
|---------|-------|------|
| Struct definition | `domain/` | `Event`, `TaskState`, `Phase` |
| Schema contract | `schemas/` | Draft 2020-12, `unevaluatedProperties: false` |
| Runtime validation | `schema_validator/` | Load schemas, validate instances |
| Reducer behavior | `domain/` | Must match schema semantics exactly |

---

## Data Flow: Create Task

```
1. CLI parses: --id, --objective, --read-scope, --write-allow, --gates
2. CLI builds CreateTaskInput (borrowed slices)
3. Application validates:
   a. No existing task with same ID
   b. Normalize paths via PathNormalizer
   c. Validate gate IDs against registry
   d. Check non-empty constraints
4. Application builds Event (seq, command_id, timestamp)
5. Application validates event against JSON Schema
6. Application appends event to events.jsonl
7. Application replays events → TaskState
8. Application writes task.json projection
9. CLI prints confirmation + next-step hint
```

**Where things go wrong**:
- Step 3a: Forgetting to check for existing task → duplicate events
- Step 3b: Skipping path normalization → escape bypass
- Step 3c: Not validating gate IDs → unknown gates in canonical events
- Step 5: Schema validation skipped when `schemas/` missing → invalid events enter log
- Step 6-7: Append succeeds but replay fails → projection out of sync

---

## Data Flow: Replay / Rebuild

```
1. Store reads events.jsonl (sorted by seq)
2. Domain reducer applies each event to fresh TaskState
3. Result is TaskState with full history
4. Store writes task.json (atomic)
```

**Critical invariant**: Same event stream → byte-identical `task.json` every time.

**Where things go wrong**:
- Non-deterministic serialization (HashMap vs BTreeSet ordering) → always use BTreeSet
- Timestamp in reducer → never; timestamps live in event envelope only
- Missing command_id dedup → events double-applied on replay

---

## Checklist for Cross-Layer Changes

### Adding a New Event Type

- [ ] Define event type string (e.g., `"task_frozen"`)
- [ ] Add `Phase` transition rule in `domain/task.rs apply()`
- [ ] Add JSON Schema case in `schemas/control.event-envelope.v1.schema.json`
- [ ] Add application command method in `application/mod.rs`
- [ ] Add CLI subcommand in `cli/mod.rs` (if exposed in current milestone)
- [ ] Add fixture in `fixtures/` covering the new transition
- [ ] Add audit matrix entry in `domain/audit_matrix.rs`
- [ ] Verify: same event stream → same projection

### Adding a New Boundary Field

- [ ] Add to `TaskState` struct (as `BTreeSet<String>`)
- [ ] Add to `CreateTaskInput` / `ReviseTaskInput`
- [ ] Add extraction in reducer's `decode_task_boundary()`
- [ ] Add to JSON Schema payload definition
- [ ] Add validation in `application` layer (non-empty, etc.)
- [ ] Add to CLI flag set
- [ ] Update all fixtures to include the new field
- [ ] Add wrong/correct examples in spec

### Adding a New Gate Template

- [ ] Add to `GATE_TEMPLATES` in `infrastructure/gates/mod.rs`
- [ ] Document in `ARCHITECTURE_GUARDRAILS.md` gate section
- [ ] Add fixture using the new gate ID
- [ ] Verify `find_template()` returns the new template
- [ ] Verify unknown gate IDs are still rejected

---

## When to Create Flow Documentation

Create detailed flow docs when:
- Feature spans 3+ layers
- Data format changes between layers
- Multiple consumers need the same data
- You're not sure where to put some logic
- You are adding an event kind, JSONL record, or config field
- UI / command code starts casting raw payload fields directly

---

## Event Log / Projection Boundary

Append-only logs are cross-layer contracts. A single event travels through:

```
CLI input → event writer → events.jsonl → reader → reducer → projection → display
```

### Checklist: After Adding A New Event Kind Or Field

- [ ] Add the event kind to the reducer's match in `apply()`
- [ ] Add to JSON Schema definition
- [ ] Make filters and reducers consume typed state, not raw JSON
- [ ] Make display code consume reducer output or typed events, not raw JSON
- [ ] Add at least one fixture that proves replay produces the same state
- [ ] Add at least one test for the illegal transition case
- [ ] Verify `processed_commands` dedup works for the new event type

---

## Common Cross-Layer Mistakes

### Mistake 1: Validation at Wrong Layer

```rust
// BAD: CLI validates business rules
if read_scope.is_empty() {
    eprintln!("read-scope must not be empty");
    return;
}

// GOOD: application layer validates
pub fn create_task(&self, id: &str, input: CreateTaskInput<'_>) -> Result<Event> {
    validate_task_definition(objective, &read_scope, &write_allow, &gates)?;
    // ...
}
```

### Mistake 2: Non-Deterministic Serialization

```rust
// BAD: HashMap iteration order is non-deterministic
pub gates: HashMap<String, GateResult>,

// GOOD: BTreeSet for ordered, deterministic output
pub read_scope: BTreeSet<String>,
```

> Note: `gate_results` uses `HashMap` which is non-deterministic. This is a known issue for M0; may need BTreeMap for replay determinism.

### Mistake 3: Display Code Parsing Raw JSON

```rust
// BAD: CLI casts payload fields directly
let obj = event.payload.get("objective").unwrap().as_str().unwrap();

// GOOD: CLI uses reducer output (TaskState)
println!("Objective: {}", state.objective.as_deref().unwrap_or("(none)"));
```
