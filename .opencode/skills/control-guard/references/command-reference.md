# Command Reference (control-guard)

On-demand depth for control-guard. The SKILL.md core carries the lifecycle and
the rules; this file is the exhaustive CLI surface — read it only when you need
a flag or a subcommand you do not remember. It is supplementary: control-guard
stays correct without it.

`ctl` is a control plane: it declares, records, and enforces. It never spawns
an executor or writes code. Every mutating command is governed by the active
task's scope + the gate.

## Task lifecycle

```text
ctl task create --id <id> --objective "<text>" \
  --read-scope <path>... --write-allow <path>... [--write-deny <path>...] \
  --gates <gate>... [--tdd] [--kind feature|research|spike]
ctl task quick --write-allow <path>            # fuse create+ready+start
ctl task ready   --id <id>                      # Planning → Ready (human gate)
ctl task approve --id <id>                      # human-only ready (proposal mode)
ctl task start  --id <id>                       # Ready → InProgress
ctl task revise --id <id> --write-allow <path>  # widen scope with approval
ctl task submit --id <id>                       # → Review (commit window opens)
ctl task finish --id <id>                       # hard-gated (audit + gates + clean tree)
ctl task archive --id <id>                      # → Completed/Cancelled → archive
ctl task status --id <id>                       # incl. audit_tier (light|full)
ctl task hold   --id <id> | ctl task resume --id <id>
```

Canonical order: **submit → record passing audit + commit in-scope work (Review) → finish → archive.**

## Scope fields (no legacy `scope`)

- `--read-scope <path>` (repeatable) — what the task may read.
- `--write-allow <path>` (repeatable) — minimal; widen only via `revise`.
- `--write-deny <path>` (repeatable) — explicit denies.
- The legacy single `scope` field is rejected everywhere.

## Gates

```text
ctl gate run    --id <id> --gate <template>     # run + record evidence
ctl gate record --id <id> --gate <gate> --passed|--failed --evidence "<text>"
ctl gate list                                   # built-in + project [[gate]] templates
```

Built-in templates: `cargo_check`, `cargo_test`, `cargo_fmt_check`, `cargo_clippy`,
`architecture_check` (Rust); `tsc_check`, `eslint_check`, `vitest_run` (Node).
Project `[[gate]]` templates in `.ctl/config.toml` extend them (same fixed
`{command, args}` shape; built-in ids reserved, collisions rejected at load).

## Review / audit

```text
CTL_ACTOR=<reviewer> ctl review accept --id <id> --note "<summary>"   # pass
CTL_ACTOR=<reviewer> ctl review reject --id <id> --note "<findings>"  # fail
```

The completion audit is **hard-gated** on finish. The recording actor must differ
from whoever started/implemented the task (M6 refuses implementer self-approval).
A prior pass is **stale** once the task is re-submitted — re-audit after rework.

## Visibility / reconciliation

```text
ctl board [--kanban|--table] [--active] [--include-archived] [--json]
ctl reconcile                                   # rebuild control.json projection
ctl replay [--task <id>]                        # replay events.jsonl → task.json
ctl validate                                    # schema-validate the ledger
ctl doctor                                      # environment + wiring health
ctl adapter doctor [--verify]                   # platform skill/hook drift + plugin tests
```

## Boundary

```text
ctl boundary check    --path <path>             # is <path> writable under the active task?
ctl boundary explain  --path <path>             # why a write was blocked
```

Protected paths (always denied outside explicit carve-outs): `.git/`,
`.ctl/tasks/*/events.jsonl`, `schemas/`, `Cargo.toml`, `Cargo.lock`. Carve-outs:
`.ctl/workflow.md`, `.ctl/scripts`, `.ctl/spec`, `.ctl/handoffs`.

## Cognitive / knowledge (record-and-disclose — never gates)

```text
ctl brainstorm record   --id <task> --convergence-path <artifact> [--divergence-path <artifact>]
ctl brainstorm attach-critic | skip-critic
ctl uncertainty record  --id <task> --id <u-id> --question "..."
ctl uncertainty resolve --id <task> --id <u-id> --disposition resolved|accepted_as_assumption --evidence <ref>
ctl research record     --id <task> --artifact <path> --kind findings|experiment|recommendation
ctl handoff export --id <id>                     # read-only task snapshot
ctl handoff capture --id <id> --file <json>      # persist agent/human judgment
```

These record evidence and disclose uncertainty; they never relax a boundary or
declare a task complete.

## Bootstrap / sync

```text
ctl init [--claude] [--opencode] [--omp] [--all] [--yes]
ctl update --merge [--force|--skip]              # sync project templates (conflict-safe)
ctl self-update [--check]                        # upgrade the binary
ctl skills sync [--check]                        # regenerate skills from .agent/skills/
ctl schema validate --file <path>
ctl architecture check | review
```
