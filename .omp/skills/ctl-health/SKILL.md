---
name: ctl-health
description: "Run full baseline health check. Bayesian diagnosis is auto-triggered by control-guard if any check fails."
---

# /ctl-health — Baseline Health Check

This skill handles ALL `ctl` commands for health verification. The main agent never calls these directly.

If any check fails, control-guard auto-triggers Bayesian reasoning to diagnose common causes. Use the diagnosis to fix the root cause, not individual symptoms.

## Execution

Run all five checks in sequence:

```powershell
cargo test
cargo clippy -- -D warnings
ctl architecture check
ctl validate
ctl doctor
```

## Results

Show a summary table:

| Check | Result |
|---|---|
| `cargo test` | ✅ N passed / ❌ <failures> |
| `cargo clippy` | ✅ clean / ❌ <warnings> |
| `architecture check` | ✅ pass / ❌ <violations> |
| `validate` | ✅ pass / ❌ <errors> |
| `doctor` | ✅ healthy / ❌ <issues> |

## If any check fails

Fix the root cause identified by Bayesian diagnosis, then re-run all checks:

- **cargo test**: Show failing test names. Fix regression before proceeding.
- **cargo clippy**: Fix warnings. `cargo clippy --fix` for auto-fixable.
- **architecture check**: Check dependency/import violations in reported files.
- **validate**: Run `ctl reconcile` to rebuild projections, re-validate.
- **doctor**: Shows which tasks have issues. Inspect their events.jsonl.

## Quick fix

Format only:
```powershell
cargo fmt
```
