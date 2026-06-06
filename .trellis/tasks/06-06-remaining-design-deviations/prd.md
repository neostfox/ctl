# Resolve remaining design deviations

## Goal

Bring the Rust control layer back into alignment with the original AI Dev Control Plane design while preserving the current milestone discipline: M0 remains a boundary/protocol freeze, and execution capabilities only re-enter through later milestone tasks after their contracts are explicit and machine-checkable.

## User Value

Maintainers should be able to progress from M0 to M3 without schema churn, duplicate canonical stores, or ambiguous task permission semantics. Every future implementation task should have a clear contract for where truth lives, what a task can read/write, how evidence becomes canonical events, and what must stop execution.

## Confirmed Facts

- Current public CLI is intentionally restricted to M0 commands: `schema`, `boundary`, and `architecture`.
- Current implementation uses `.control/events.jsonl` and `.control/<task>.json` internally, but README's original design describes `.trellis/tasks/<task>/events.jsonl`, `telemetry.jsonl`, `task.json`, and `control.json`.
- Original MVP command list targets M3 Manual loop, not M0.
- Current `task_created`/`task_revised` payloads use a simplified `scope` array plus `gates`; the original design calls for distinct read/write/deny/risk boundaries.
- Approval, scoped lease, evidence ingest, deterministic audit, human resume, and full completion interlock remain design goals, not active M0 capabilities.
- Telemetry, drift, next-action, schedule, and OMP adapter are intentionally post-MVP / later milestone concerns.
- Current schema/docs now consistently use `hold_entered`, `hold_exited`, and `gate_checked`.
- User decision: M1+ canonical store path is `.trellis/tasks/<task>/events.jsonl`; `.control` must not remain a second canonical store.
- User decision: M1+ task boundary model is structured: `read_scope`, `write_allow`, `write_deny`, `risk_triggers`, and `gates`.
- User decision: use a parent-plus-children Trellis structure. This parent task owns cross-milestone decisions; child tasks own M1, M2, and M3 implementation.

## Requirements

1. Decide and document the canonical task store path before M1 implementation resumes.
2. Decide and document the task boundary model before restoring `task create` / `task revise` commands.
3. Split remaining work into milestone-safe Trellis tasks so M1/M2/M3 do not silently reintroduce M4+ concepts.
4. Keep M0 public CLI restricted until an implementation task explicitly advances the milestone and passes architecture checks.
5. Ensure every future execution command has a fail-closed schema, reducer behavior, fixture coverage, and architecture check before it becomes public.
6. Preserve `events.jsonl` as canonical truth; projections remain rebuildable outputs.
7. Do not introduce async, network, database, or adapter-specific bindings before their roadmap milestone.

## Acceptance Criteria

- [ ] A canonical store decision is recorded in this task's `design.md` and reflected in future schema/fixtures before implementation starts.
- [ ] A task boundary contract is recorded in `design.md` with explicit read/write/deny/gate/risk semantics.
- [ ] `implement.md` contains an ordered milestone-safe task split from current M0 to M3.
- [ ] Open questions are resolved or explicitly left as blocked product decisions.
- [ ] No code implementation starts until the user approves the planning artifacts or asks to proceed.

## Out of Scope For This Planning Task

- Implementing M1/M2/M3 commands immediately.
- Building telemetry/drift/controller logic.
- Implementing OMP/Codex/Claude/OpenCode adapters.
- Adding new dependencies.
- Changing the current M0 CLI surface during planning.

## Open Questions

1. Canonical store path: RESOLVED — use `.trellis/tasks/<task>/events.jsonl` with colocated projections and planning artifacts.
2. Task boundary model: RESOLVED — use structured `read_scope` / `write_allow` / `write_deny` / `risk_triggers` / `gates` from M1 onward.
3. Task split: RESOLVED — keep this task as parent/planning and create child tasks for M1 canonical ledger, M2 gate policy, and M3 manual loop.
