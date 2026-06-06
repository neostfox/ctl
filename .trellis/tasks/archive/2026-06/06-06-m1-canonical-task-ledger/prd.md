# M1 canonical task ledger

## Goal

Restore local task-ledger capabilities on the approved canonical store: `.trellis/tasks/<task>/events.jsonl`.

## Parent Task

`.trellis/tasks/06-06-remaining-design-deviations`

## Confirmed Decisions

- Canonical truth is `.trellis/tasks/<task>/events.jsonl`.
- `.control` must not remain a second canonical store.
- M1 task boundary model is structured: `read_scope`, `write_allow`, `write_deny`, `risk_triggers`, and `gates`.
- M0 public CLI remains restricted until M1 commands are implemented with schema, reducer, fixture, and architecture coverage.

## Requirements

1. Refactor store code to use `.trellis/tasks/<task>/events.jsonl` as append-only canonical truth.
2. Generate `task.json` as replay projection in the same task directory.
3. Do not write canonical task events to `.control`.
4. Upgrade `task_created` and `task_revised` payload schemas to structured boundaries.
5. Update reducer state to store normalized `read_scope`, `write_allow`, `write_deny`, `risk_triggers`, and `gates`.
6. Reintroduce only M1-safe CLI commands after contracts exist: task create/revise/ready/status/replay/validate/doctor as applicable.
7. Architecture check must fail on dual canonical store exposure or legacy `scope` usage.

## Acceptance Criteria

- [x] `task create` rejects empty objective, empty `read_scope`, empty `write_allow`, empty `gates`, protected paths, and unknown gates before append.
- [x] `task revise` rejects legacy `scope` and preserves structured boundary invariants.
- [x] Replay regenerates identical `task.json` from the same event stream.
- [x] `.control/events.jsonl` is not created or used by M1 public commands.
- [x] `cargo fmt --check`, `cargo check`, `cargo test`, `cargo clippy -- -D warnings`, and `cargo run -- architecture check` pass.

## Out of Scope

- Gate command execution policy.
- Assignment export/import.
- Evidence ingest.
- Completion interlock beyond reducer-level task state invariants.
- Telemetry, drift, schedule, or automated adapters.
