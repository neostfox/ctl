# PRD: Attestation layer — subagent-dispatch + run attestation

> Status: **confirmed** (2026-06-20) · informs tasks, does not gate them · supersedes nothing
> Source context: `brainstorms/orchestration-trust-audit-v1.md` (Findings 2, 3, 7 / B2)

## Objective

Turn the two "substrate-vs-attestation" gaps the orchestration-trust audit found
into **honest, durable ledger records**:

1. a subagent dispatch becomes a canonical fact (role, adapter, parent task/run,
   instruction/context/output hashes);
2. a production run actually reaches `Finished` on its ledger and carries
   provenance (hashes, model/provider, started/ended/exit).

The reframing that makes this buildable without the forbidden crypto deps
(**confirmed U-A**): **ctl records host-supplied provenance as evidence and
hashes the artifacts it is given (`sha2`) — it does not cryptographically prove
what ran.** Same record-and-disclose posture as the existing epistemic layer; no
overclaiming. Every recorded hash/field is disclosed as *host-attested, not
ctl-verified*.

## Context

- **ObservedBasis** (the committed audit, which cites file:line):
  - *Finding 7*: the `task`-tool dispatch is gated but **never appended to the
    ledger** (verdict-only), and **no dispatch hash** exists; OMP keeps only an
    in-memory spawn timestamp.
  - *Finding 2*: `AgentRunState` (`domain/run.rs:60-95`) has **no**
    hash/role/model/provider/timestamp/exit fields.
  - *Finding 3 / B2*: `run_finished`/`finish_run` has only `#[cfg(test)]`
    callers, `run_failed` none, run-scoped `gate_checked`/`evidence_*` are never
    emitted — a prod run **never reaches Finished** on the ledger.
- **ConfirmedBasis**: design-first chosen; these two scoped as the buildable set.
  Dependency guardrail (`AGENTS.md` Forbidden): deps capped at
  `clap/serde/anyhow/sha2/libc`, no HTTP client — so `sha2` content-hashing is
  in-bounds, signing/OIDC is not. **U-A** = record-and-disclose accepted.
  **U-B** = schema changes go through the `ctl apply` reviewed-exception flow.
- **Non-goals**: `authenticated-principal-v1`, `trusted-orchestrator-envelope-v1`
  (crypto/HTTP — blocked); cryptographic *proof* of execution; ctl spawning
  executors.

## Resolved uncertainties

- **U-A** → record-and-disclose (host-attested evidence, `sha2` artifact hashes).
- **U-B** → `ctl apply` reviewed exception for the specific `schemas/` files.
- **U-C** (run-scoped gate/evidence emission) → **deferred + documented**, not
  emitted in this pass (the audit allowed either); the run lifecycle reaching
  `Finished` is the priority.
- **U-D** (host wiring) → **primitives first**; OMP/opencode wiring is a separate
  later slice.

## Tasks (vertical slices)

- **id: run-finish-emit-v1** — wire the reducer-ready `run_finished` into
  production (`RunCommands::Finish` + application emit) so a run reaches
  `Finished` and recover/replay stop seeing every prod run as open. **No schema
  change** (event already exists) → unblocked regardless of U-B. AFK-safe.
  write-allow: `src/cli/mod.rs, src/application/mod.rs`; gates: floor.
- **id: run-attestation-fields-v1** — add host-supplied provenance to the run
  aggregate + `run_finished` event (instruction/context/output hash via
  `--artifact` paths ctl hashes; model/provider/started/ended/exit), disclosed
  host-attested. **Schema change via `ctl apply`.** AFK-safe.
  write-allow: `src/domain/run.rs, src/application/mod.rs, schemas/<run-event>`;
  gates: floor.
- **id: subagent-dispatch-record-v1** — new canonical `subagent_dispatched`
  event (role, adapter, parent task/run, instruction/context/output hashes from
  supplied artifacts) + pure reducer arm + `ctl dispatch record` command.
  **Schema change via `ctl apply`.** AFK-safe.
  write-allow: `src/domain/*, src/application/mod.rs, src/cli/mod.rs, schemas/<dispatch-event>`;
  gates: floor.
- **id: dispatch-attestation-host-wiring-v1** *(deferred / U-D)* — OMP/opencode
  call `ctl dispatch record` on spawn. Lands after the primitives.

Each task: `sha2`-only, TDD (red evidence before green), carries its
host-attested disclosure, and never relaxes a boundary.
