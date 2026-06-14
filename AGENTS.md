<!-- TRELLIS:START -->
# AI Dev Control Plane — Agent Instructions

## Project Identity

- **Name**: `ai-dev-control-plane`
- **Binary**: `control` (Rust CLI, `src/main.rs` → `cli::run()`)
- **Purpose**: Local-first, deterministic AI development control layer. Governs task lifecycle, boundary enforcement, and gate validation for AI-assisted coding.
- **NOT**: A model executor, remote orchestration platform, daemon, or web service.

## Architecture

```
src/
  cli/              → clap CLI parsing, architecture checks
  application/      → ControlApp command service; validates before event append
  domain/           → Pure reducer (apply) + TaskState + Event; no I/O
  infrastructure/
    store/          → FileEventStore (events.jsonl read/append, task.json projection)
    boundary/       → PathNormalizer (reject escape, UNC, symlinks, protected paths)
    gates/          → GateTemplate registry + M0 stub runner
    schema_validator.rs → JSON Schema validation against schemas/
  adapters/manual/  → Reserved for M3 manual adapter
```

**Dependency direction** (inviolable):
```
cli → application → domain
infrastructure/* → domain
adapters/manual → application DTO
```

`domain/` MUST NOT depend on `cli/`, `infrastructure/`, `adapters/`, filesystem, network, time, or processes.

## Canonical Truth Model

```
events.jsonl   = append-only canonical truth (per task, under .ctl/tasks/<id>/)
telemetry.jsonl = append-only evidence index (cross-task, .ctl/; M5)
task.json      = replay projection (delete and rebuild, never hand-edit)
control.json   = reconcile projection — per-task board + M5 drift/next-action decision
```

- External actors (agents, adapters, humans) CANNOT append canonical events directly.
- Telemetry, agent output, and human backfill are **evidence**, not state. M5 drift
  rules read telemetry as a signal; an unknown signal fails closed (never relaxes scope).

## Milestone Gate

Current milestone scope: **M0–M3**.

| Milestone | Focus | Key Constraint |
|-----------|-------|----------------|
| M0 | Boundary protocol freeze | Schema + boundary + architecture checks |
| M1 | Local task ledger | `events.jsonl` CRUD lifecycle |
| M2 | Validation, boundary, archive | Scope check, gate runner, finish interlock |
| M3 | Manual closed-loop MVP | `manual` adapter, assignment/evidence flow |

**Hard stops** (ARCHITECTURE_GUARDRAILS.md):
- M3 before any auto-agent execution.
- No async runtime, HTTP client, database, Web UI, or daemon before M3.
- No `AgentRun` aggregate before M6.

## Key Conventions

1. **Events**: Strict ascending `seq`, idempotent `command_id`, schema `control.event-envelope.v1`.
2. **Reducer**: Pure function `apply(&mut TaskState, &Event)`. No side effects. State machine: `Planning → Ready → InProgress → Review → Completed` (plus `Cancelled`).
3. **Hold**: Orthogonal to phase. Violation, gate failure, or human pause triggers hold. No `start`/`submit`/`finish` while held.
4. **Gates**: Only predefined templates (`cargo_check`, `cargo_test`, `cargo_fmt_check`, `cargo_clippy`). M0 does NOT execute gates — runner is stubbed.
5. **Paths**: Normalized before boundary checks. Reject absolute, `..`, UNC, symlinks, junctions, root-escape, protected paths (`.git/`, `.ctl/tasks/*/events.jsonl`, `schemas/`, `Cargo.toml`).
6. **Legacy `scope` field**: Must be rejected everywhere. Use `read_scope` + `write_allow` + `write_deny`.

## Spec & Documentation

- `.ctl/spec/backend/` — Layer-specific coding guidelines.
- `.ctl/spec/guides/` — Cross-cutting thinking guides.
- `ARCHITECTURE_GUARDRAILS.md` — Inviolable architecture rules.
- `ROADMAP.md` — Milestone definitions, exit criteria, and decisions.
- `schemas/` — JSON Schema contracts (Draft 2020-12, `unevaluatedProperties: false`).

## Commands (M0–M1 Surface)

```text
ctl init
ctl task create --id <id> --objective <text> --read-scope <path> --write-allow <path> --gates <gate>
ctl task revise --id <id> [boundary fields]
ctl task ready --id <id>
ctl task status --id <id>
ctl replay [--task <id>]
ctl validate
ctl doctor
ctl schema validate --file <path>
ctl boundary check --path <path>
ctl boundary explain --path <path>
ctl architecture check
```

## Forbidden

- Modifying `events.jsonl` entries in-place (append-only).
- Writing `task.json` by hand or via agent.
- Adding dependencies beyond: `clap`, `serde`/`serde_json`, `anyhow`. (See DEP-001..DEP-004.)
- Introducing `tokio`, `reqwest`, or any async runtime before M3.
- Skipping phase transitions (e.g., `Planning → InProgress` without `Ready`).

Managed by Trellis. Edits outside this block are preserved; edits inside may be overwritten by a future `trellis update`.
<!-- TRELLIS:END -->
