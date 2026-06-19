# Release Notes — ctl v0.0.4

Follows **v0.0.3**. `ctl --version` reports the built version from
`CARGO_PKG_VERSION`; the release tag must equal `Cargo.toml` (enforced by
`release.yml`), and the npm `@ai-dev/ctl` meta-package plus its five platform
packages carry the matching version.

This is a factual changelog. It contains no scores or quality grades.

## Included since 0.0.3

- **Project default gate floor.** `ctl task create` / `ctl task quick` no longer
  require `--gates`; when omitted they derive the floor from
  `.ctl/config.toml [project].default_gates`. There is no hardcoded floor in ctl —
  the `ctl-spec-bootstrap` skill analyzes the project and records the floor (it
  mirrors what the project enforces: correctness + lint + formatting gates). A
  project with no floor and no `--gates` errors clearly.
- **Full Claude Code support.** `ctl init --platform <claude|opencode|omp|all>`
  selects the integration to inject (interactive when no flag + TTY; the flag is
  required in a non-interactive shell). `.claude/` now ships the governance hooks,
  the control-guard entry/router, the six workflow skills + cli-reference, a
  `CLAUDE.md` managed block with **read-only subagent dispatch routing**, and a
  read-only `ctl-oracle` diagnostician agent.
- **Single-sourced workflow skills + `ctl skills sync`.** Each workflow skill has
  one source at `.agent/skills/<skill>/source.md` (frontmatter + shared body +
  per-platform integration); `ctl skills sync` generates every platform's
  `SKILL.md`, and `ctl skills sync --check` fails CI on drift. The managed core
  stays byte-identical to the canonical protocol and the body identical across
  platforms by construction.
- **`ctl-cli-reference` skill** — a lifecycle-focused reference for the ctl CLI so
  agents read docs instead of probing `--help`.
- **`ctl prd init`** (PRD scaffold), **`ctl ralph`** (bounded read-only safety
  supervisor for unattended runs — never spawns an executor or writes code), and
  **`ctl handoff export`** (read-only portable task snapshot).
- **TTL-gated run-lease expiry** and **`ctl repair --cross-ledger`** (detect/repair
  task↔run inconsistencies; preview by default, `--apply` to act).
- **M6 shared-`.git` hardening** — destructive git ops are denied while a run is
  active.
- **`.claude/config.toml` carve-out** — the project config is AI-writable under
  governance (the canonical `.ctl/tasks` ledger stays protected); the
  `PlatformSkill` model is decoupled from the executor-adapter registry so Claude
  Code can host a drift-checked control-guard without being an adapter.

## Behavior notes

- **Subagent dispatch under ctl (`.claude/subagent-dispatch.md`).** Read-only work
  (investigation, search, research) is dispatched to read-only subagents; **writes
  stay inline** in the main agent, which alone carries the active task's
  `CTL_TASK_ID` binding and routes its tool calls through the gate. Whether a
  subagent's tool calls reach the PreToolUse gate at all is a **host** behavior
  that ctl does not control and treats as unverified; writable subagent roles are
  therefore deferred. This is a disclosed posture, not a proof.

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
