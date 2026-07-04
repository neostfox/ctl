# M3 Dogfood Findings

Captured from 2026-06-07 M3 dogfood session (10 real tasks).

## Windows Compatibility

### Gate runner requires SystemRoot, TEMP, TMP

`link.exe` (MSVC linker used by `cargo test`) requires `SystemRoot`, `TEMP`, `TMP`, `OS`, and `PROCESSOR_*` env vars. The gate runner's `build_allowed_env()` must pass these through, otherwise `cargo_test` gate will always FAIL on Windows while `cargo_check` passes (check doesn't link).

**Source**: `src/infrastructure/gates/mod.rs` — `build_allowed_env()`

### Path separator mismatch in scope check

`PathNormalizer::normalize()` returns `\`-separated paths on Windows. CLI `--write-allow` args use `/`. Scope comparison in `ingest_manual_result()` must normalize both sides to `/` before comparing.

**Source**: `src/application/mod.rs` — `ingest_manual_result()`

## Schema

### Event types must be added before use

If M3 code emits an event type not in `schemas/control.event-envelope.v1.schema.json` enum, `validate_and_append()` will reject it. Always add new event types AND their payload schemas before shipping code that emits them.

**Example**: `evidence_accepted`, `evidence_rejected` were used in code but missing from schema enum.

## Completion Interlock

### Rejected evidence must be resolvable

The completion interlock must NOT count all historical `evidence_rejected` events. Instead, track which files were rejected and resolve them when a subsequent `evidence_accepted` covers that file. Otherwise, any transient ingest error permanently blocks task completion.

**Source**: `src/application/mod.rs` — `finish_task()`

## PathNormalizer

### Protected paths block read scope too

`is_protected()` applies to both read and write scope paths. This prevents tasks from tracking schema changes. The protection should only apply to write scope, not read scope.

**Status**: Known gap, not yet fixed.
