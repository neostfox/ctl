---
name: control-guard
description: "Agent-driven control plane integration. The agent automatically detects task boundaries, proposes structured task creation, enforces write scope during implementation, and verifies gates before completion."
---

# Control Guard — Agent-Driven Control Plane

You are operating inside an OMP agent session with the `control` CLI available on PATH. This skill teaches you to weave the control plane into every phase of your work automatically — the human should never need to run `control` commands manually.

## Core Loop

```text
Human request
  → Agent classifies: is this a task?
    → Yes: Agent proposes boundaries → asks human → creates control task
    → No: Skip control, work freely
  → Agent implements within scope
  → Agent auto-verifies gates
  → Agent checks completion readiness
```

## When to Engage

Engage the control plane when the human's request meets ANY of these:

1. Involves modifying source files (not just reading/explaining)
2. Has a clear objective that can be verified (tests pass, feature works)
3. Touches more than one file or one module
4. The human describes a feature, bug fix, or refactoring

Skip the control plane when:

1. Pure conversation, explanation, or question
2. Reading or searching without modification intent
3. The human explicitly says "skip control" or "just do it"

## Phase 1: Automatic Task Proposal

When you detect a task-worthy request, DO NOT immediately start coding. Instead:

### Step 1.1: Infer boundaries

Analyze the request and codebase to infer:

- `objective`: One clear sentence of what will be achieved.
- `read_scope`: Which directories/files the agent will need to read.
- `write_allow`: Which directories/files the agent will modify.
- `write_deny`: Paths that must NOT be touched (protected files, config, unrelated modules).
- `risk_triggers`: What could go wrong (dependency changes, schema changes, breaking API).
- `gates`: Which verification commands must pass.

Infer gates based on project type:
- Rust project: `cargo_fmt_check`, `cargo_check`, `cargo_test`, `cargo_clippy`
- The default set for this project: `cargo_fmt_check`, `cargo_check`, `cargo_test`, `cargo_clippy`

### Step 1.2: Present proposal to human

Show the inferred task boundary in a compact format and ask for confirmation:

```
📋 Control task proposal:

  Objective: Fix config parsing to handle nested TOML
  
  Read:    src/config/, tests/config/
  Write:   src/config/, tests/config/
  Deny:    .env, Cargo.toml, schemas/
  Risks:   existing config backward compat
  Gates:   cargo_fmt_check, cargo_check, cargo_test, cargo_clippy
  
  Approve? (yes / adjust / skip)
```

Wait for human response:

- `yes` → proceed to Step 1.3
- `adjust` → ask what to change, update proposal, re-present
- `skip` → work without control plane for this request

### Step 1.3: Create control task

```bash
# Ensure the control ledger exists
control init

# Create the task with the approved boundary
control task create \
  --id "<slug>" \
  --objective "<objective>" \
  --read-scope <path> \
  --write-allow <path> \
  --write-deny <path> \
  --risk-triggers <trigger> \
  --gates <gate_id>
```

Then mark ready:

```bash
control task ready --id "<slug>"
```

Store the task id for the rest of the session. All subsequent control commands reference this id.

## Phase 2: Scoped Implementation

### Before every file write

Before modifying any file, check:

```
Is the target path within write_allow?
  → Yes: proceed
  → No: STOP. Tell the human this path is outside scope.
    Ask: extend scope, or find an in-scope approach?
```

You can verify a path against the current task scope:

```bash
control boundary check --path <target_path>
```

If the path is rejected and you believe it should be in scope, ask the human. Do NOT silently write outside scope.

### During implementation

After each meaningful change (file saved, test passing), you MAY run:

```bash
control validate
```

This checks that the event stream is still clean.

### Tracking touched files

Maintain a mental list of every file you modify. You will need this list for completion verification.

## Phase 3: Gate Verification

When you believe implementation is complete, automatically run ALL required gates:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
```

If any gate fails:

1. Fix the issue
2. Re-run the failed gate
3. Do NOT declare completion until ALL gates pass

After all gates pass:

```bash
control architecture check
```

## Phase 4: Completion Proposal

When all gates pass, present a completion summary:

```
✅ Task "<slug>" completion summary:

  Objective: Fix config parsing to handle nested TOML
  
  Files modified:
    - src/config/parser.rs
    - tests/config/parser_test.rs
  
  Gates:
    ✓ cargo_fmt_check
    ✓ cargo_check
    ✓ cargo_test
    ✓ cargo_clippy
  
  Scope check: all modifications within write_allow
  
  Final status:
    control task status --id "<slug>"
```

Ask the human: "Mark as verified?"

### After human confirms

```bash
control validate
control replay --task "<slug>"
```

The task stays in `ready` phase (we do not have `start/submit/finish` in M1). The human can inspect the final state with:

```bash
control task status --id "<slug>"
```

## Multi-Task Sessions

If the human's request contains multiple independent deliverables:

1. Propose a parent task + children structure
2. Create the parent first
3. Create each child with its own scope
4. Work through children one at a time
5. Each child gets its own gate verification cycle

## Recovery

### Aborting a task

If the human says to abandon the current task:

```bash
control task status --id "<slug>"
```

Record the state and move on. The task stays in the ledger for audit purposes.

### Extending scope mid-task

If during implementation you discover the write_allow is too narrow:

1. STOP modifying files
2. Tell the human what additional path is needed and why
3. Wait for approval
4. Create a revised task (if CLI exposes revise) or ask the human to create a new task

## Anti-Patterns

- ❌ Never run `control` commands silently without showing the human
- ❌ Never modify files outside `write_allow` without explicit human approval
- ❌ Never skip gate verification because "it probably passes"
- ❌ Never declare completion with a failing gate
- ❌ Never create a control task for pure conversation or read-only requests
- ❌ Never use `--scope` (legacy); always use `--read-scope` / `--write-allow`

## Integration with Trellis

When Trellis is active in the same project:

1. The control task id should match or reference the Trellis task slug
2. The control task boundary should be reflected in the Trellis PRD
3. Gate verification results feed into the Trellis check phase
4. The control `events.jsonl` is the canonical timeline; Trellis markdown is human-readable planning

Suggested Trellis PRD addition:

```markdown
## Control Layer Boundary

Task ID: `<slug>`
Write scope: `<write_allow paths>`
Gates: `<gate list>`
Verify with: `control task status --id <slug>`
```
