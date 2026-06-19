---
name: ctl-cli-reference
description: "Lifecycle-focused reference for the ctl CLI — the governed task lifecycle (create → ready → start → submit → commit → gate → review → finish → archive), plus the commands and key flags for gates, reviews, out-of-scope exceptions, evidence/provenance, inspection, and recovery. Read this instead of probing the surface with `--help`; fall back to `ctl <command> --help` only for exhaustive flag detail. Use when you need to drive a ctl task end to end or look up which command/flag does what."
---

# ctl CLI reference (lifecycle-focused)

`ctl` is the control plane: it owns facts, scope, gates, evidence, and the
canonical ledger. This skill covers the ~80% of the surface agents use daily.
For exhaustive flags on any command, run `ctl <command> --help` — but read here
first instead of probing blind.

Conventions: every task is identified by `--id <id>` (maps to `.ctl/tasks/<id>/`).
Most write commands accept `--dry-run` (validate + show, persist nothing).

## The governed task lifecycle (the spine)

```
create ──▶ ready ──▶ start ──▶ [implement] ──▶ submit ──▶ [commit] ──▶ gate run* ──▶ review accept ──▶ finish ──▶ archive
                                                  │                                    (distinct actor)
   Planning          InProgress              Review (commit window opens here)        Completed     archived
```

- **`ctl task create --id <id> --objective <text> --read-scope <p>… --write-allow <p>… [--gates <id>…] [--write-deny <p>…] [--risk-triggers <t>…] [--depends-on <id>…] [--tdd] [--kind implementation|research]`**
  Create a Planning task with a structured boundary. If `--gates` is omitted it
  derives the project floor from `.ctl/config.toml [project].default_gates`;
  with no floor and no `--gates` it errors. `--tdd` enforces a red→green
  cargo_test interlock at finish.
- **`ctl task quick --write-allow <p>… [--id <id>] [--objective <t>] [--read-scope <p>…] [--gates <id>…] [--depends-on <id>…]`**
  Fuses create + ready + start for small changes. Same gate derivation as create.
- **`ctl task revise --id <id> [--objective] [--read-scope…] [--write-allow…] [--gates…] [--depends-on…] …`**
  Edit a **Planning** task's boundary; omitted fields keep current values.
- **`ctl task ready --id <id>`** → mark Planning task Ready.
- **`ctl task start --id <id>`** → Ready → InProgress (now writes are gated to scope).
- **`ctl task submit --id <id>`** → InProgress → Review. **This opens the git
  commit window** — commit your in-scope work only after submit, before finish.
- **`ctl task reopen --id <id>`** → Review → InProgress (to rework).
- **`ctl task finish --id <id>`** → Review → Completed. Completion interlock: all
  required gates passing (latest result), a fresh completion audit recorded after
  the last submit, working tree clean within `write_allow`; research tasks also
  need a recorded research artifact + an uncertainty disposition.
- **`ctl task cancel --id <id>`** → cancel a non-terminal task.
- **`ctl task archive --id <id>`** → archive a Completed/Cancelled task.
- **`ctl task status --id <id> [--json]`** → current projection (phase, gates, scope).

## Gates

- **`ctl gate run --id <id> --gate <template>`** — execute a gate template and
  record the result as a canonical event (bind it to the committed tree; run
  after submit + commit). Templates: `cargo_check`, `cargo_test`, `cargo_clippy`,
  `cargo_fmt_check`. Only allowlisted templates run (no arbitrary shell).
- **`ctl gate record --id <id> --gate <t> --passed <bool> --evidence <text>`** —
  record an externally-verified gate result.

## Review & out-of-scope exceptions

- **`ctl review accept --id <id> [--note <text>]`** — record a **passing**
  completion audit (M-f). Must be a **distinct actor** from the implementer:
  `CTL_ACTOR=ctl-review ctl review accept …` (self-approval is refused when the
  implementer is the default `human` actor).
- **`ctl review reject --id <id> --note <text>`** — record a failing audit
  (blocks finish until reworked + re-audited). Allowed from any actor.
- **`ctl apply --id <id> --path <p> --reason <text> [--ttl <secs>]`** — request a
  reviewed exception to write **one path outside** `write_allow` (or a protected
  path). Files a path-scoped approval.
- **`ctl approval grant --id <id> --request <request_id>`** — grant a filed
  request (issue as `CTL_ACTOR=ctl-review`; opens that path at the gate).
- **`ctl approval request --id <id> --action <a> --reason <text>`** — request a
  step-up approval for a gated action (e.g. `deps` for `cargo install`).

## Evidence & provenance (record-only — these never render a verdict)

- **`ctl audit --id <id>`** — generate the deterministic audit report.
- **`ctl research record --id <id> --kind findings|experiment|recommendation|design-draft --artifact <path>`**
  — record a research artifact (research tasks require ≥1 before finish).
- **`ctl uncertainty record --id <id> --uncertainty <U-001> --statement <text> [--source <note>]`**
  then **`ctl uncertainty dispose --id <id> --uncertainty <U-001> --disposition resolved|accepted-as-assumption|invalidated [--evidence <path> | --evidence-ref <id>] [--reason <text>]`**
  — open and terminally dispose an unknown (research tasks require ≥1 disposition).
  `ctl uncertainty status --id <id>` shows the ledger; `ctl uncertainty evidence`
  records an oracle-typed evidence object a `resolved` disposition can reference.
- **`ctl brainstorm …`** — record which cognitive artifacts a task derived from.
- **`ctl telemetry …`** — submit drift signals (M5 evidence index).

## Inspection (read-only)

- **`ctl report`** — summary of all tasks. **`ctl board`** — cross-task control
  board (phase/hold/gates/review per task + totals).
- **`ctl doctor`** — ledger health. **`ctl validate`** — validate event logs.
- **`ctl architecture check`** — architecture/boundary compliance.
- **`ctl handoff export --id <id>`** — portable read-only task snapshot for
  another session/human (emits no events).
- **`ctl drift …`** / **`ctl next-action`** — deterministic drift analysis and an
  advisory next step (pass/ask/stop/replan/rescope); emit no events.

## Recovery & projection

- **`ctl replay [--task <id>]`** — rebuild task.json projection(s) from events.
- **`ctl reconcile`** — rebuild all task views (writes `.ctl/control.json`).
- **`ctl repair [--task <id> | --run <id> | --all] [--cross-ledger] [--apply]`** —
  truncate a torn trailing record, or detect/repair task↔run inconsistencies
  (preview by default; `--apply` to act).

## Runs, workspace, scheduling, adapters

- **`ctl run …`** (assignment export + result ingest, M3/M4),
  **`ctl workspace …`** (worktree isolation, M4),
  **`ctl schedule …`** (concurrent multi-task execution, M6),
  **`ctl adapter capabilities --adapter <name>`**,
  **`ctl agent-report`**. See each `--help` for the run/ingest flow.

## Governance rules every agent must know

- **Protected paths** (`.git`, `.ctl`, `.ctl/tasks`, `schemas`, `Cargo.toml`,
  `Cargo.lock`) cannot be in `write_allow`. Carve-outs: `.ctl/config.toml`,
  `.ctl/workflow.md`, `.ctl/scripts`, and `.ctl/spec/**` (always gate-allowed).
  To touch another protected/out-of-scope path, use `ctl apply` + `approval grant`.
- **Commit window**: `git commit` is only allowed after `ctl task submit` (Review)
  — the PreToolUse gate inspects the whole command, so submit in a *separate* step
  before committing.
- **Finish needs a fresh, independent audit**: re-run gates and re-audit after the
  last submit; the audit actor must differ from the implementer.
- **Multiple active write tasks** → the gate returns `multiple_active` and fails
  closed unless the call is bound via `CTL_TASK_ID` / `--task <id>`.
- **Fail closed**: when ctl is unavailable, Write/Edit are denied (Bash is not, to
  avoid locking out the shell).

> For any flag not listed here, run `ctl <command> --help`. Prefer reading this
> reference over probing the CLI surface command-by-command.
