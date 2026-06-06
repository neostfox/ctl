# Backend Development Guidelines

> Coding conventions for the `ai-dev-control-plane` Rust CLI project.

---

## Overview

This project is a **local-first, deterministic AI development control layer** implemented as a single Rust workspace with one CLI binary (`control`). The codebase is organized into strict layers with enforced dependency direction.

**Key invariant**: `domain/` is pure — no filesystem, network, time, or process access. All side effects live in `infrastructure/` or `cli/`.

**Truth model**: `events.jsonl` is the single append-only canonical truth. `task.json` is a replay projection. External actors cannot append canonical events directly.

---

## Architecture Layers

```
cli/          → CLI parsing (clap derive), output formatting, architecture checks
application/  → ControlApp: command validation, event building, orchestration
domain/       → Pure reducer + state types + event definitions
infrastructure/
  store/      → FileEventStore: JSONL read/append, task.json write
  boundary/   → PathNormalizer: escape/protected path rejection
  gates/      → GateTemplate registry, M0 stub runner
  schema_validator.rs → JSON Schema Draft 2020-12 validation
adapters/
  manual/     → Reserved for M3 manual adapter
```

**Dependency direction** (MODULE-001..MODULE-005):

```
cli → application → domain
infrastructure/* → domain
adapters/manual → application DTO
```

Violations: `domain/` importing from `cli/`, `infrastructure/`, or `adapters/` is a hard STOP.

---

## Pre-Development Checklist

Before writing code in any layer, read the corresponding spec:

- [ ] Read the **layer spec** for the target module (domain, application, infrastructure, cli)
- [ ] Check `ARCHITECTURE_GUARDRAILS.md` for milestone-scoped rules
- [ ] Verify the change belongs to the current milestone (M0–M3 only)
- [ ] Confirm no new runtime dependency is needed (DEP-001..DEP-004)

---

## Quality Check

After implementation:

- [ ] `cargo check` passes
- [ ] `cargo test` passes (including fixture-based reducer tests)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] No new `domain/` dependency on infrastructure or I/O
- [ ] New event types have corresponding reducer branches and fixture coverage
- [ ] Path-like inputs pass through `PathNormalizer` before entering policy decisions

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module layout, event store contracts, projection paths | Filled |
| [Storage Guidelines](./storage-guidelines.md) | FileEventStore, JSONL append, projection writes, atomic replace | Filled |
| [Error Handling](./error-handling.md) | anyhow patterns, validation error matrix, user-facing messages | Filled |
| [Quality Guidelines](./quality-guidelines.md) | Forbidden patterns, required patterns, testing requirements | Filled |
| [Logging & Output](./logging-output-guidelines.md) | CLI output conventions, exit codes, structured diagnostics | Filled |
| [Domain Layer](./domain-layer.md) | Reducer purity, state machine, event contracts | Filled |
| [Infrastructure Layer](./infrastructure-layer.md) | Store, boundary normalizer, schema validator, gates | Filled |
| [CLI Layer](./cli-layer.md) | Command surface, clap patterns, output formatting | Filled |

---

**Language**: All documentation is written in **English**.

**References**: All rules map to IDs in `ARCHITECTURE_GUARDRAILS.md` (e.g., `MODULE-001`, `STATE-001`).
