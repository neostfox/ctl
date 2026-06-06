# M3 manual loop MVP

## Goal

Complete the manual execution loop so the OMP agent can autonomously verify scope compliance, ingest evidence, and enforce the completion interlock — making the full agent-driven cycle operational without human gate checks.

## Parent Task

`.trellis/tasks/06-06-remaining-design-deviations`

## Dependency

M1 canonical task ledger (done) and M2 gate policy/boundary enforcement (must be complete).

## Confirmed Decisions

- Canonical store is `.trellis/tasks/<task>/events.jsonl`.
- Task boundary uses structured fields: `read_scope`, `write_allow`, `write_deny`, `risk_triggers`, `gates`.
- OMP agent-driven workflow is active via `.omp/skills/control-guard/SKILL.md`.
- Evidence is not canonical truth — it is validated by the control layer before canonical events are emitted.

## Requirements

1. Implement `control task submit --id <task>` to transition InProgress → Review, automatically checking:
   - All required gates have latest passing result.
   - All touched files are within `write_allow`.
   - No touched files are in `write_deny`.
   - No pending holds.
2. Implement `control task finish --id <task>` to transition Review → Completed, enforcing full completion interlock:
   - Required gates latest passed.
   - Touched files within scope.
   - No pending approvals, active holds, expired/revoked/exhausted leases.
   - Baseline manifest has not regressed.
3. Implement `control task archive --id <task>` for Completed/Cancelled → Archived.
4. Manual adapter: `control assignment export --id <task>` generates a structured assignment JSON from task boundary + context snapshot.
5. Manual adapter: `control run ingest --id <task> --adapter manual --result <file>` validates and ingests manual execution output as evidence.
6. Generate deterministic audit report from events + evidence.
7. Update `.omp/skills/control-guard/SKILL.md` to automate: scope diff check, evidence ingest, completion interlock, and audit report generation.

## Acceptance Criteria

- [ ] `task submit` fails closed on missing gates, out-of-scope files, or active holds.
- [ ] `task finish` fails closed on any unmet interlock condition.
- [ ] Assignment export contains complete boundary + context for external execution.
- [ ] Manual result ingest rejects malformed evidence and out-of-scope touched files.
- [ ] Audit report is deterministic from event/evidence inputs.
- [ ] Agent skill covers full cycle: create → ready → start → implement → submit → finish → archive.
- [ ] Standard Rust verification gates pass.

## Out of Scope

- OMP adapter (direct API integration).
- Automated reviewer.
- Telemetry/drift/controller.
- Scheduling and next-action automation.
