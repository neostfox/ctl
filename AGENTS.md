# AI Dev Control Plane Development Context

## Project Intent

Build a local-first Rust CLI for controlled AI-assisted development.

The product is a deterministic control layer, not a model executor and not a remote orchestration platform.

Read these documents before making architecture decisions:

- `ARCHITECTURE_GUARDRAILS.md`
- `ROADMAP.md`
- `README.md`

## Current Milestone

Work is currently limited to `M0: boundary protocol freeze`.

Allowed M0 work:

```text
Rust CLI skeleton
JSON Schema definitions
domain event envelope
Task state transition table
pure reducer fixtures
Windows path normalization design and tests
architecture check design
```

Deferred until later milestones:

```text
automatic agent execution
OMP adapter runtime
telemetry scoring
drift automation
multi-agent scheduling
network services
database
daemon
Web UI
```

## Architecture

Use one Rust workspace and one CLI binary during `M0-M3`.

```text
src/
  main.rs
  cli/
  application/
  domain/
  infrastructure/
    store/
    boundary/
    gates/
  adapters/
    manual/
schemas/
fixtures/
tests/
```

Dependency direction:

```text
cli -> application -> domain
infrastructure/* -> domain
adapters/manual -> application DTO
main.rs = composition root
```

The `domain/` layer must remain deterministic and side-effect free. It must not directly access files, Git, processes, network, or current time.

## Canonical State

```text
events.jsonl     = append-only canonical truth
telemetry.jsonl  = evidence only
task.json        = replay projection
control.json     = reconcile projection
Markdown         = human explanation only
```

External tools submit evidence. They do not append canonical events directly.

## Change Discipline

Before editing:

1. State the active milestone.
2. Identify affected guardrail IDs.
3. List any new dependency, module, schema, or side effect.
4. Ask before changing a `REVIEW` boundary.
5. Stop when a `STOP` boundary would be violated.

Do not commit, push, merge, install dependencies, or access the network through shell commands unless the user explicitly approves the exact action.

## Verification

When a Rust project exists, prefer:

```text
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
```

Run focused checks first. Report commands that could not be run.

