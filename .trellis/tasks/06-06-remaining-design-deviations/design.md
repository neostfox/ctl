# Design: Resolve Remaining Design Deviations

## Current State

M0 is intentionally a protocol/boundary freeze. The current codebase is now safer than the earlier broad CLI implementation because it does not expose task execution, gate execution, context build, ingest, replay, reconcile, or adapter paths as public commands.

The remaining design gap is not a failing M0 command; it is that future milestones need a stable contract before execution features are reintroduced.

## Decision Area 1: Canonical Store Path

### Original Design

README describes a Trellis-compatible task store:

```text
.trellis/tasks/<task>/
  events.jsonl
  telemetry.jsonl
  task.json
  control.json
  prd.md
  design.md
  implement.md
  implement.jsonl
  check.jsonl
  research/
```

It also states:

```text
events.jsonl     = append-only canonical truth
telemetry.jsonl  = append-only evidence index
task.json        = replay projection
control.json     = reconcile projection
```

### Current Implementation Direction

Existing store code uses `.control/events.jsonl` and `.control/<task>.json`. Public M0 CLI does not expose that store, so the mismatch is contained.

### Decision

Adopt `.trellis/tasks/<task>/events.jsonl` as the canonical store for M1+.

Rationale:

- It matches the original Trellis-compatible design.
- It avoids a second task truth source beside Trellis task artifacts.
- It makes PRD/design/implement/research colocated with canonical events.
- It allows `task.json` and `control.json` to become explicit projections in the same task directory.

Trade-off:

- Requires migrating existing internal `FileEventStore` assumptions before M1 commands return.
- Requires architecture checks to protect `.trellis/tasks/**/events.jsonl` and generated projection semantics carefully.

Status: accepted by user during planning.

Fallback Option

Keep `.control` as native store and update README/ROADMAP to make Trellis only a planning layer. This is simpler short-term but loses Trellis-compatible storage and increases risk of duplicate task state.

## Decision Area 2: Task Boundary Contract

### Original Design

The README and guardrails emphasize that each task must declare:

- target objective
- read scope
- write allowlist
- write denylist / protected paths
- required gates
- risk triggers / review triggers

### Current Simplification

Current event payloads use:

```json
{
  "objective": "...",
  "scope": ["..."],
  "gates": ["..."]
}
```

This is too coarse for assignment generation and completion interlock.

### Decision

Use a structured task boundary model for M1+:

```json
{
  "objective": "...",
  "read_scope": ["..."],
  "write_allow": ["..."],
  "write_deny": ["..."],
  "risk_triggers": ["..."],
  "gates": ["cargo_fmt_check", "cargo_check", "cargo_test", "cargo_clippy"]
}
```

Rules:

- `read_scope`, `write_allow`, and `gates` require `minItems: 1`.
- `write_deny` defaults to protected paths and may add task-specific deny entries.
- Boundary normalizer validates all path-like fields before events append.
- Reducer stores normalized boundaries, not raw unchecked inputs.
- Public commands must not accept legacy `scope` once the new contract is active.

Status: accepted by user during planning.

Trade-off:

- More verbose CLI and fixtures.
- Better fail-closed behavior and fewer migrations before M3.

## Decision Area 3: Milestone-Safe Reintroduction

Reintroduce execution capabilities only in this order:

1. M1 local task ledger: canonical store, task create/revise/ready/status/replay, projections.
2. M2 gates and boundary enforcement: deterministic gate templates, runner policy, evidence events.
3. M3 manual loop: assignment export/import, manual adapter ingest, audit report, completion interlock.

Do not reintroduce:

- approval / lease automation before its schema and reducer are frozen
- telemetry / drift / next-action before later milestones
- OMP adapter before manual adapter proves the protocol

## Compatibility / Migration Notes

- Existing fixtures can be upgraded in-place because they are development fixtures, not user data.
- Current `.control` store code can be refactored before exposure; no public migration is required while M0 CLI hides it.
- Architecture checks should pin either `.trellis/tasks/**/events.jsonl` or `.control/events.jsonl`, never both as canonical truth.

## Rollback Shape

If the `.trellis/tasks` store decision proves too coupled to Trellis scripts, roll back to `.control` only by updating README/ROADMAP/guardrails first, then changing store code and architecture checks. Do not support dual canonical writes.
