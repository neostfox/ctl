<!-- TRELLIS:START -->
# AI Dev Control Plane — Agent Instructions

## Project Identity

- **Name**: `ai-dev-control-plane`
- **Binary**: `ctl` (Rust CLI, `src/main.rs` → `cli::run()`)
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
    gates/          → GateTemplate registry + live gate runner (cargo_check/test/fmt/clippy)
    workspace/      → worktree isolation, diff/apply changesets (M4)
    skills/         → embedded skills/hooks injected by `ctl init`
    schema_validator.rs → JSON Schema validation against schemas/
  adapters/         → manual (M3), omp + opencode (M4) executor adapters
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

Current scope (0.0.3): **M0–M6 shipped + hard review layer (M-a…M-g) + V1 cognitive layer**.

| Milestone | Focus | Key Constraint |
|-----------|-------|----------------|
| M0 | Boundary protocol freeze | Schema + boundary + architecture checks |
| M1 | Local task ledger | `events.jsonl` CRUD lifecycle |
| M2 | Validation, boundary, archive | Scope check, gate runner, finish interlock |
| M3 | Manual closed-loop MVP | `manual` adapter, assignment/evidence flow |
| M4 | Isolated single-executor run | worktree + scoped lease + approval; omp/opencode adapters |
| M5 | Explainable control loop | telemetry evidence, drift engine, `ctl board`, `next-action` |
| M6 | Restricted multi-agent | AgentRun aggregate, concurrent runs, crash/merge recovery, lease wiring |
| M-a…M-g | Hard review layer | multi-active gate, `control.json`, cross-task overlap/deps, dispatch binding, hard review gate, commit interlock |
| V1 | Cognitive layer | brainstorm / uncertainty / research / handoff / prd / ralph (record-and-disclose) |

**Invariant (unchanged across all milestones)**: `ctl` NEVER spawns an executor or writes
code — it plans, governs, and ingests results; an external executor (OMP/opencode) drives runs.

**Standing hard stops** (ARCHITECTURE_GUARDRAILS.md):
- No async runtime, HTTP client, database, Web UI, or daemon in the `ctl` core.
- Dependencies stay minimal (see Forbidden below).

## Key Conventions

1. **Events**: Strict ascending `seq`, idempotent `command_id`, schema `control.event-envelope.v1`.
2. **Reducer**: Pure function `apply(&mut TaskState, &Event)`. No side effects. State machine: `Planning → Ready → InProgress → Review → Completed` (plus `Cancelled`).
3. **Hold**: Orthogonal to phase. Violation, gate failure, or human pause triggers hold. No `start`/`submit`/`finish` while held.
4. **Gates**: Only predefined templates (Rust: `cargo_check`, `cargo_test`, `cargo_fmt_check`, `cargo_clippy`; TypeScript/Node: `tsc_check`, `eslint_check`, `vitest_run`). The gate runner executes them and records evidence; a timed-out gate's process tree is terminated without hanging the supervisor.
5. **Paths**: Normalized before boundary checks. Reject absolute, `..`, UNC, symlinks, junctions, root-escape, protected paths (`.git`, `.ctl`, `.ctl/tasks`, `.control`, `schemas`, `Cargo.toml`, `Cargo.lock`) — with carve-outs for `.ctl/workflow.md` and `.ctl/scripts`.
6. **Legacy `scope` field**: Must be rejected everywhere. Use `read_scope` + `write_allow` + `write_deny`.
7. **Gate observe mode**: the host write gate (`ctl hook gate`) allows-and-records out-of-scope / task-less mutations and out-of-window commits/pushes to the non-canonical `.ctl/decisions.jsonl`, returning a model-visible `warning`; protected paths, deps step-up, held tasks, cross-task overlap, and multi-active ambiguity remain hard denies. See `.ctl/spec/prd/gate-observe-mode.md`.

## Spec & Documentation

- `.ctl/spec/backend/` — Layer-specific coding guidelines.
- `.ctl/spec/guides/` — Cross-cutting thinking guides.
- `ARCHITECTURE_GUARDRAILS.md` — Inviolable architecture rules.
- `ROADMAP.md` — Milestone definitions, exit criteria, and decisions.
- `schemas/` — JSON Schema contracts (Draft 2020-12, `unevaluatedProperties: false`).

## Agent Workflow Skills

A ctl-native **workflow foundation** lives alongside the control-guard skill. The
canonical core is `.agent/protocols/workflow-skills.md` (`WORKFLOW_PROTOCOL_VERSION = 1`),
embedded verbatim in each skill across OMP (`.omp/skills/`) and OpenCode
(`.opencode/skills/`); a CI drift test (`workflow_protocol_sync` in
`infrastructure/skills.rs`) refuses to let the copies or the platform-shared phase
bodies diverge.

```
grill  → ctl-grill-with-spec   align from first principles (challenge inherited
                               assumptions; output artifacts, not truth)
PRD    → ctl-to-prd            synthesize a PRD; separate ObservedBasis /
                               ConfirmedBasis / OpenUncertainty (draft|confirmed|superseded)
tasks  → ctl-to-tasks          vertical, independently verifiable slices; each
                               declares scope, gates, AFK/HITL, blocking uncertainties
TDD    → ctl-tdd-loop          one behavior, red→green (the `--tdd` / `tdd-red-green`
                               interlock proves it on the ledger)
handoff→ ctl-handoff           compact context (builds on `ctl handoff export`)
```

These are **agent workflow disciplines**, not new governance. They do not prove
correctness, do not replace gates / audits / evidence, do not create authenticated
reviewer independence, and do not create L3 tamper evidence. The frameworks are
placed, not floating: **First Principles** in grill, **Bayesian reasoning** in
`ctl-diagnose`. External inspiration (Matt Pocock's skills; Trellis PR #335) is L0
reference material — adapted, never vendored (see `.omp/skills/NOTICE.md`).
Follow-ups deferred: a `ctl-architecture-review` skill (the `ctl architecture
review` CLI already exists) and a diagnose-v2 skill.

## Commands (core subset — run `ctl --help` for the full surface)

The full CLI spans task lifecycle, gates, context, assignment, run/workspace/approval (M4),
schedule/agent-report (M6), telemetry/drift/next-action (M5), review/apply (hard gate), and the
V1 layer (brainstorm/uncertainty/research/handoff/prd/ralph). Core entry points:

```text
ctl init
ctl task create --id <id> --objective <text> --read-scope <path> --write-allow <path> --gates <gate>
ctl task quick --write-allow <path>          # fuse create+ready+start
ctl task ready|start|submit|finish|archive --id <id>
ctl gate run --id <id> --gate <template>
ctl board [--json]                            # cross-task control board
ctl replay [--task <id>] | ctl validate | ctl doctor
ctl schema validate --file <path>
ctl boundary check|explain --path <path>
ctl architecture check
```

## Forbidden

- Modifying `events.jsonl` entries in-place (append-only).
- Writing `task.json` by hand or via agent.
- Adding dependencies beyond: `clap`, `serde`/`serde_json`, `anyhow`, `sha2`, and `libc` (unix-only, for process-group signalling). (See DEP-001..DEP-004.)
- Introducing `tokio`, `reqwest`, or any async runtime into the `ctl` core.
- Skipping phase transitions (e.g., `Planning → InProgress` without `Ready`).

Managed by Trellis. Edits outside this block are preserved; edits inside may be overwritten by a future `trellis update`.
<!-- TRELLIS:END -->
