---
name: control-guard
description: "Agent-driven control plane integration. The agent automatically detects task boundaries, proposes structured task creation, enforces write scope during implementation, runs gates, checks boundaries, and verifies completion interlock."
---

# Control Guard — Agent-Driven Control Plane (M3)

You are operating inside an OMP agent session with the `control` CLI available on PATH. This skill teaches you to weave the control plane into every phase of your work automatically — the human should never need to run `control` commands manually.

## Core Loop (M3)

```text
Human request
  → Agent classifies: is this a task?
    → Yes: Agent proposes boundaries → asks human → creates control task
    → No: Skip control, work freely
  → Agent starts task
  → Agent builds context snapshot
  → Agent exports assignment for external execution
  → Agent implements within scope (or external executor does)
  → Agent ingests execution results as evidence
  → Agent runs gates through control layer
  → Agent checks boundaries
  → Agent submits for review
  → Agent generates audit report
  → Agent verifies completion interlock
  → Agent finishes and archives
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

Then mark ready and start:

```bash
control task ready --id "<slug>"
control task start --id "<slug>"
```

Store the task id for the rest of the session. All subsequent control commands reference this id.

## Phase 2: Context Build

After starting the task, immediately build a context snapshot:

```bash
control context build --id "<slug>"
```

This hashes all files in the task's `read_scope` into `.trellis/tasks/<slug>/context.json`. The boundary checker will use this baseline to detect modifications.

## Phase 2b: Assignment Export (M3)

After context build, export a structured assignment for the executor (manual or future adapter):

```bash
control assignment export --id "<slug>"
```

This writes `.trellis/tasks/<slug>/assignment.json` containing the full boundary + context snapshot. The assignment file is the contract between the control layer and any executor.

## Phase 2c: Result Ingest (M3)

After execution (manual or by external tool), ingest the result as evidence:

```bash
control run ingest --id "<slug>" --adapter manual --result <result_file>
```

The result file must be valid JSON with:

```json
{
  "source": "manual",
  "touched_files": ["src/main.rs", "tests/test_foo.rs"],
  "exit_code": 0,
  "summary": "What was done"
}
```

The control layer validates:
- `source` must be `"manual"` for the manual adapter
- All `touched_files` must be within `write_allow` and not in `write_deny`
- Malformed results are rejected with `evidence_rejected` events
Accepted results generate `evidence_accepted` canonical events.

## Phase 3: Scoped Implementation

### Before every file write

Before modifying any file, check:

```
Is the target path within write_allow?
  → Yes: proceed
  → No: STOP. Tell the human this path is outside scope.
    Ask: extend scope, or find an in-scope approach?
```

You can validate a path against the current task scope:

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

Maintain a mental list of every file you modify. You will need this list for boundary verification.

## Phase 4: Gate Verification

When you believe implementation is complete, run ALL required gates through the control layer:

```bash
control gate run --id "<slug>" --gate cargo_fmt_check
control gate run --id "<slug>" --gate cargo_check
control gate run --id "<slug>" --gate cargo_test
control gate run --id "<slug>" --gate cargo_clippy
```

Each gate execution is recorded as a canonical `gate_checked` event. The control layer runs gates with EXEC-002 controls: bounded output, environment allowlist, and 60-second timeout.

If any gate fails:

1. Fix the issue
2. Re-run the failed gate
3. Do NOT declare completion until ALL gates pass

After all gates pass:

```bash
control architecture check
```

## Phase 5: Boundary Check

After all gates pass, check boundaries:

```bash
control boundary check-by-id --id "<slug>"
```

This compares the current workspace against the context snapshot. Any file modified outside `write_allow` generates a `boundary_violation_recorded` event and puts the task on hold.

If violations are detected:

1. STOP. The task is now on hold.
2. Tell the human what files were modified outside scope.
3. Wait for human direction: adjust scope, revert changes, or create a new task.

## Phase 6: Audit and Completion (M3)

When all gates pass AND boundary check is clean:

### Step 6.1: Generate audit report

```bash
control audit --id "<slug>"
```

This produces a deterministic audit report from events + evidence. The report includes:
- Gate results and pass/fail status
- Evidence accepted/rejected counts
- Boundary violation count
- Completion interlock verdict (allow/blocked/completed)

The report is written to `.trellis/tasks/<slug>/audit-report.json`.

### Step 6.2: Submit for review

```bash
control task submit --id "<slug>"
```

The submit command now checks:
- No active hold
- No boundary violations recorded
- Phase must be InProgress

Present a completion summary:

```
✅ Task "<slug>" completion summary:

  Objective: Fix config parsing to handle nested TOML
  
  Files modified:
    - src/config/parser.rs
    - tests/config/parser_test.rs
  
  Evidence: 1 accepted, 0 rejected
  
  Gates:
    ✓ cargo_fmt_check
    ✓ cargo_check
    ✓ cargo_test
    ✓ cargo_clippy
  
  Scope check: all modifications within write_allow
  
  Boundary check: clean
  Audit interlock: allow
```

Ask the human: "Mark as verified?"

### Step 6.3: After human confirms

```bash
# Human verifies the change and finishes the task
control task finish --id "<slug>"

# Optionally archive
control task archive --id "<slug>"
```

The `finish` command enforces full completion interlock:
- Phase must be `review`
- No active hold
- All required gates must have latest passing results
- No rejected evidence

## M3 Command Reference

```text
control init                                    # Initialize ledger
control task create --id --objective ...        # Create task
control task revise --id [--objective ...]       # Revise in Planning
control task ready --id                         # Planning → Ready
control task start --id                         # Ready → InProgress
control task status --id                        # Print task view
control task submit --id                        # InProgress → Review (checks hold/violations)
control task reopen --id                        # Review → InProgress
control task finish --id                        # Review → Completed (full interlock)
control task cancel --id                        # → Cancelled
control task archive --id                       # terminal → archived
control context build --id                      # Hash read_scope files
control assignment export --id                  # Export structured assignment JSON (M3)
control run ingest --id --adapter manual --result <file>  # Ingest manual result as evidence (M3)
control audit --id                              # Generate deterministic audit report (M3)
control report                                  # Summary of all tasks (M3)
control boundary check --path <path>            # Validate a single path
control boundary check-by-id --id               # Check task workspace diff
control boundary explain --path <path>          # Explain path decision
control gate run --id --gate <gate_id>          # Execute gate via EXEC-002
control gate record --id --gate --passed --evidence  # Record external result
control replay [--task <id>]                    # Rebuild projections
control reconcile                               # Rebuild all projections
control validate                                # Validate event logs
control doctor                                  # Diagnose ledger health
control architecture check                      # Architecture compliance
```

## Multi-Task Sessions

If the human's request contains multiple independent deliverables:

1. Propose a parent task + children structure
2. Create the parent first
3. Create each child with its own scope
4. Work through children one at a time
5. Each child gets its own context build, gate verification, and boundary check cycle

## Recovery

### Aborting a task

If the human says to abandon the current task:

```bash
control task cancel --id "<slug>"
```

### Task on hold (boundary violation or gate failure)

If the task enters hold due to boundary violation:

1. The `boundary_violation_recorded` event automatically puts the task on hold
2. You CANNOT continue writing while on hold
3. Ask the human to resolve: revert the violation, or adjust scope via new task

### Extending scope mid-task

If during implementation you discover the write_allow is too narrow:

1. STOP modifying files
2. Tell the human what additional path is needed and why
3. Wait for approval
4. Cancel the current task and create a new one with expanded scope (MVP does not support scope expansion after Ready per STATE-014)

## Anti-Patterns

- ❌ Never run `control` commands silently without showing the human
- ❌ Never modify files outside `write_allow` without explicit human approval
- ❌ Never skip gate verification because "it probably passes"
- ❌ Never declare completion with a failing gate
- ❌ Never skip boundary check before submitting for review
- ❌ Never create a control task for pure conversation or read-only requests
- ❌ Never use `--scope` (legacy); always use `--read-scope` / `--write-allow`
- ❌ Never bypass `control gate run` by running cargo commands directly — results must be recorded through the control layer

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
