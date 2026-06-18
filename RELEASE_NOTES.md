# Release Notes — ctl v0.0.3

Follows **v0.0.2** (published 2026-06-16). `ctl --version` reports the built
version from `CARGO_PKG_VERSION`; the release tag must equal `Cargo.toml`
(enforced by `release.yml`), and the npm `@ai-dev/ctl` meta-package plus its five
platform packages carry the matching version.

This is a factual changelog. It contains no scores or quality grades.

## Included since 0.0.2

- **OpenCode executor adapter + governed session** — `ctl run ingest --adapter
  opencode`, `.opencode/plugins/ctl-gate.ts` gate plugin (Bun-tested in CI).
- **Shared adapter conformance suite** — one contract asserted over every
  registered adapter (`omp`, `opencode`); a half-wired adapter fails CI.
- **Single managed control-guard protocol core + CI drift check** — the canonical
  protocol (`.agent/protocols/control-guard.md`) is embedded verbatim in each
  platform skill; drift fails CI.
- **`ctl adapter list / status / doctor`** (adapter-doctor-v1) — read-only
  diagnostics along two axes: `contract.*` (the runtime twin of the conformance
  suite) and `platform.*` (skill presence, managed-protocol drift reusing the CI
  checker, plugin/hook/config presence, opencode Bun tests under `--verify`).
  Factual per-check status (`PASS / FAIL / WARN / UNKNOWN / NOT_TRACKED`) and
  counts — **no composite health score**.
- **Same-task E2E parity** (adapter-same-task-e2e-v1) — a deterministic,
  model-free harness (`scripts/adapter_parity_e2e.sh`) drives the identical task
  lifecycle through both adapters and confirms parity across lifecycle,
  write-scope enforcement, evidence source, gate binding, audit, finish/archive,
  and adapter-doctor before/after. See `ADAPTER_PARITY_E2E.md`.
- **Executor policy version bound into `policy_hash`** — see below.

## Behavior notes

- **Executor policy version in `policy_hash`.** `policy_hash` now folds in
  `EXECUTOR_POLICY_VERSION` (the run-manifest contract + ingest scope check +
  `validate_output` generation). Bumping it makes prior gate/audit evidence
  stale (its `policy_hash` no longer matches `finish`'s recomputed value),
  forcing re-run under the new executor policy. Replay is unaffected — recorded
  hashes are read, never recomputed.
- **Legacy model-backed resolved uncertainties.** A `model` oracle is ADVISORY
  and **cannot** resolve an unknown; new model-backed resolve is rejected at the
  command layer. Ledgers written before that rule may contain model-backed
  `resolved` uncertainties — they remain replayable (append-only) and are
  **disclosed as ADVISORY**, never as external proof.
- **Durability.** Events are appended with an explicit `flush()` + `sync_all()`
  (fsync) per line; a torn trailing record (e.g. from a crash mid-append) is
  detected on read and repaired explicitly via `ctl repair` (dry-run by default,
  `--apply` to truncate the torn tail) and surfaced by `ctl doctor`.

## Known limitations / non-claims

These are deliberate boundaries, not TODOs to silently close:

- **No authenticated principal.** "reviewer ≠ implementer" is enforced by `actor`
  **label** only: audits/approvals are recorded under a distinct `CTL_ACTOR`
  (e.g. `ctl-review`), which is a reviewer **role label, not a proven independent
  identity**. Do not read it as "independently/authentically approved".
- **Write boundaries are not OS sandboxing.** They are fail-closed interceptions
  at the agent tool-hook layer (OMP / Claude Code / opencode). A process that
  does not route through a hook is not constrained by them.
- **The event log is not L3 tamper-evident.** No hash chain, signature, or
  external anchoring. `tree_hash` / `policy_hash` / `evidence_hash` are
  content/envelope-integrity hashes, not cryptographic attestations of identity
  or claim truth.
- **Concurrent task/run orchestration is experimental and not cross-ledger
  atomic.** Single-writer ordering holds per task/run ledger, but a task
  transition and its run-ledger counterpart are separate appends; a crash
  between them can leave the ledgers disagreeing. `ctl doctor` surfaces the
  inconsistency and the manual recovery step — ctl never auto-rewrites state.
- **Windows process-tree termination on gate timeout is best-effort.** It uses
  `taskkill /PID <pid> /T /F` (no Job Object), confirmed by reaping the managed
  root process — a grandchild spawned mid-sweep is not guaranteed reaped. Unix
  uses process-group signalling (`kill(-pgid, …)`, TERM→KILL). On both, a gate
  whose tree cannot be confirmed terminated is reported as an execution
  containment failure, never as an ordinary failed gate.

## Verification

`cargo fmt --check`, `cargo check/clippy/test --locked`, and
`ctl architecture check` gate every push (`ci.yml`); the release workflow
(`release.yml`) re-runs the same verification and `build`/`release` jobs
`needs:` it, so a release cannot ship code that fails the correctness gate.
