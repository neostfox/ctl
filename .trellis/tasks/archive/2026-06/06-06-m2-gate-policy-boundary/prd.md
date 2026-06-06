# M2 gate policy and boundary enforcement

## Goal

Reintroduce gate and boundary enforcement only after EXEC-002 controls are explicit and fail-closed. Enable the OMP agent to automatically execute gates and detect scope violations during implementation.

## Parent Task

`.trellis/tasks/06-06-remaining-design-deviations`

## Dependency

M1 canonical task ledger is complete and archived.

## Confirmed Decisions

- Canonical store is `.trellis/tasks/<task>/events.jsonl`.
- Task boundary uses structured fields: `read_scope`, `write_allow`, `write_deny`, `risk_triggers`, `gates`.
- OMP agent-driven workflow is active via `.omp/skills/control-guard/SKILL.md`.

## Requirements

1. Implement `gate run` with EXEC-002 controls: timeout (60s default), environment allowlist (PATH + CARGO_* only), output cap (64KB), deterministic command templates, explicit working directory.
2. Reintroduce CLI commands: `control gate run --id <task> --gate <gate_id>` and `control context build --id <task>`.
3. Context build snapshots all files in `read_scope` with hash into `.trellis/tasks/<task>/context.json`.
4. Boundary enforcement compares touched paths (from context diff) against `write_allow` and `write_deny`; violations generate `boundary_violation_recorded` events.
5. Gate results are recorded only through control-layer-validated canonical `gate_checked` events.
6. CLI reintroduces `task start` (Planning → InProgress) as the agent's entry point after ready.
7. Update `.omp/skills/control-guard/SKILL.md` to automate gate execution and context building.
8. Required gate baseline changes must trigger architecture check failure until reviewed.

## Acceptance Criteria

- [x] `gate run` executes only allowlisted command templates with bounded output and no network access.
- [x] `context build` produces deterministic hash snapshot of read scope files.
- [x] Boundary check detects files modified outside `write_allow` and records violations.
- [x] Unknown gates fail schema/app/reducer checks.
- [x] `task start` transitions Planning → InProgress.
- [x] Agent skill updated to auto-run gates and context build.
- [x] Standard Rust verification gates pass.

## Out of Scope

- Manual assignment export/import.
- Automated model adapters.
- Drift scoring and scheduler.
- Completion interlock beyond gate results.
