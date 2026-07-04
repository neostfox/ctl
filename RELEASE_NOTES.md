# Release Notes — ctl v0.0.10

Follows **v0.0.9**. `ctl --version` reports `CARGO_PKG_VERSION`; the release tag
must equal `Cargo.toml` (enforced by `release.yml`), and the npm `@velo-ai/ctl`
meta-package plus its five platform packages carry the matching version (stamped
to the tag at publish time).

This is a factual changelog. It contains no scores or quality grades.

## Included in 0.0.10 — Pipeline stations: grill v2 + handoff contracts

- **`ctl-grill-with-spec` v2 is the alignment station's single entry.** The
  phase body restores the micro-decision interview the original skill lineage
  (mattpocock/skills `grilling`) had: one question at a time, each carrying a
  recommended answer; "facts from the repo, direction from the user" replaces
  the old "only ask the user for what the repo cannot answer" stance;
  **nothing is built until the user confirms shared understanding**. Broad
  requests get an explicit diverge step (2-3 candidate approaches with
  trade-offs). The alignment note lands at a fixed path
  `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md` (spec tier — writable under the
  gate) and is registered as record-only brainstorm provenance.
- **`ctl-brainstorm` is retired to an alias stub** (`.omp`, `npm-omp`) pointing
  at grill v2 — its interview loop was absorbed, removing the duplicate
  alignment entry and the Claude/OMP skill asymmetry.
- **Station contracts.** `ctl-grill-with-spec`, `ctl-to-prd`, and
  `ctl-to-tasks` now open with an explicit contract: upstream artifact →
  produces → downstream consumer (alignment note → PRD → task proposals), so
  each stage consumes the previous stage's artifact instead of floating free.
  Sources regenerated via `ctl skills sync` (12 files, incl. npm-omp mirrors).
- **control-guard core v3: Pipeline Routing (proposal-first).** The routing
  section now names the six stations (triage → align → PRD → tasks → execute →
  wrap-up), requires reporting the current station when routing, and encodes
  the complexity ladder: trivial edits skip the pipeline; everything else gets
  a proposal + micro-decision confirmation before `ctl task create`. Synced
  across `.agent/protocols/` + 4 platform copies; complexity-classification
  guide updated to match.
- **`ctl task create`/`quick` print a non-blocking provenance hint** pointing
  at `ctl brainstorm record` when a task should link back to its alignment
  note (record-only — creation is never gated).
- **`cargo install --path <dir>` reclassified** from `cargo_deps` to
  `cargo_build`: a self-install of the local crate (the dev-loop's binary
  reinstall) is not a dependency change. Registry installs
  (`cargo install <crate>`) still require the deps step-up approval.

## Included in 0.0.10 — Gate observe mode (deny → record + warn)

- **The write gate no longer denies by default.** `ctl hook gate` verdicts for
  out-of-scope Write/Edit (including out-of-repo targets), writes with no
  `in_progress` task, `bash_write` while idle, and `git commit`/`git push`
  outside the Review window change from `allowed:false` to `allowed:true` +
  `record:true` + a `warning` field. Observed calls land in the non-canonical
  `.ctl/decisions.jsonl` (the gate-decision-log-v1 channel); the Claude
  PreToolUse hook forwards the warning to the model as `additionalContext`
  **without** a permission decision, so the user's normal permission flow is
  untouched. ADR: `.ctl/spec/prd/gate-observe-mode.md`.
- **Protected paths are now denied explicitly, not incidentally.** A new
  gate-side classifier (`classify_write_target`) resolves the hook's absolute
  target against the project root and checks the boundary-normalizer protected
  list (`.git`, `.ctl` ledgers, `.control`, `schemas/`, `Cargo.toml`,
  `Cargo.lock`, with the `.ctl/config.toml` / `workflow.md` / `scripts`
  carve-outs). Previously protection fell out of "never in write_allow", which
  observe mode would have silently softened. A granted `ctl apply` exception
  still authorizes a specific protected path; traversal/UNC targets the gate
  refuses to classify stay denied.
- **The hard core is unchanged**: deps changes without a `deps` step-up
  approval, held tasks, cross-task write overlap (M-c), multi-active ambiguity
  without a dispatch binding, destructive git ops during active runs (M6), and
  fail-closed for Write/Edit/MultiEdit when `ctl` is unavailable.
- **Protocol + docs synced**: control-guard core bumped to
  `CONTROL_GUARD_PROTOCOL_VERSION = 2` across `.agent/protocols/` and all four
  platform skill copies (`.claude`, `.omp`, `.opencode`, `npm-omp`);
  `ctl-context.py` session context and AGENTS.md describe the observe posture.
- **Known gap**: the opencode plugin allows-and-records observed verdicts but
  does not yet surface the `warning` text to the model.

## Included in 0.0.10 — TypeScript / Node gate templates

- **ctl gains non-Rust gate templates.** The gate registry
  (`src/infrastructure/gates/mod.rs`) previously shipped only the four `cargo_*`
  templates, so non-Rust projects (Node/TypeScript/Python/Go/Java) had **no**
  enforceable gate: `ctl task create` without `--gates` errored on the missing
  floor, and there was no valid non-Rust gate id to pass either. Three
  TypeScript/Node templates are added, invoked via `npx` so they resolve to the
  project's local `node_modules/.bin`:
  - `tsc_check` — `npx tsc --noEmit` (type-check)
  - `eslint_check` — `npx eslint .` (lint)
  - `vitest_run` — `npx vitest run` (tests)
  EXEC-003's network denylist means a missing tool **fails closed** (the registry
  fetch is denied) rather than silently installing — so these gates honestly
  require the tool to be installed as a devDependency.
- **`/ctl-spec-bootstrap` records a real floor for TypeScript projects.** Step
  1.5's template table now lists the TS gates as applicable; projects detected as
  TypeScript get `default_gates = ["tsc_check", "eslint_check", "vitest_run"]`
  instead of an empty floor.
- **Fixture/known-gate lists synced.** `check_fixture_paths_gates`
  (`src/cli/mod.rs`) and the `test_list_templates_count` / `test_find_template_known`
  tests now include the three new ids.
- **AGENTS.md gate list** updated to declare the TS templates.

### Known gaps deferred (tracked separately)

- The TDD red→green interlock (`application::TDD_TEST_GATE`) is still bound to
  `cargo_test`; a TypeScript TDD task would need `vitest_run`.
- Python / Go / Java still have no templates.
- A generic "run arbitrary command" gate (which would let a project define its
  own without a code change) needs a gate data-model change and is not included.


## Included since 0.0.7 — `ctl init` OMP integration verification & idempotency

- **`ctl init` reachability check mirrors the hook and execs the binary.** The
  post-init self-check resolved `ctl` by `is_file()` alone and omitted the
  local-`node_modules` step the OMP hook checks first (`resolveBundledCtl`), so
  it could warn "binary not found" on a machine where the hook resolves `ctl`
  fine — e.g. after `omp plugin link` or a local `npm i @velo-ai/omp`, where the
  binary lives under the project's `node_modules`. It now mirrors the hook's full
  order — `CTL_BIN` → bundled `@velo-ai/ctl*` (`@velo-ai/ctl-<plat>` or
  `@velo-ai/ctl/platforms/<plat>`) → global npm → `~/.cargo/bin` → real exe on
  PATH — and actually runs the resolved binary (`--version`, 5s-bounded) to prove
  it is executable, not merely present.
- **`ctl init` surfaces the OMP plugin-link prerequisite.** For `--platform omp`
  / `all`, init now verifies the governance hook file is present and prints that
  OMP loads the hook only from an npm-installed or `omp plugin link`-ed plugin —
  a marketplace install (`omp plugin install github:…`) does **not** load the
  extension, so governance silently never fires. This is the one prerequisite
  `ctl init` cannot detect itself.
- **`ctl init` is idempotent for `.omp/settings.json`.** Re-running init no
  longer rewrites an existing `settings.json` when the control-guard autoLoad
  entry is already present. Previously `merge_settings` re-serialized the file on
  every init; since `serde_json` sorts object keys (no `preserve_order`
  feature), that reordered the user's settings into alphabetical order — a noisy
  diff for an already-correct config. The merge is now a true no-op when nothing
  needs adding.

## Included since 0.0.6 — @velo-ai npm org, OMP plugin package & Windows hook fix

- **npm org renamed `@ai-dev` → `@velo-ai`.** The meta-package is now
  `@velo-ai/ctl`, with five `@velo-ai/ctl-<platform>` binary packages. The
  `@ai-dev` org was unavailable; every reference (wrapper error text, OMP hook
  lookups, plugin generator, docs) moved in lockstep.
- **`@velo-ai/omp` — installable OMP plugin.** `ctl skills sync` now also
  generates `npm-omp/` (the `@velo-ai/omp` package) from the canonical `.omp/`
  source: a `package.json` declaring the OMP extension entry (the governance
  hook) plus a dependency on `@velo-ai/ctl`, so `npm i` / `omp plugin link`
  installs the integration **and** the platform binary together. A cargo drift
  test (`omp_plugin_package_is_in_sync_on_disk`) fails CI if the package drifts
  from its source.
- **PATH-independent `ctl` resolution in the OMP hook.** The pre-hook resolved
  `ctl` by bare name against the host process PATH, which fails on Windows when
  `ctl` is installed off the launch PATH — the gate then fails closed and blocks
  every mutating tool. It now resolves `CTL_BIN` → the bundled `@velo-ai/ctl`
  package (`require.resolve`) → `~/.cargo/bin/ctl[.exe]` → bare `ctl`.
- **npm publish pipeline.** `release.yml` gains an `npm-publish` job: it stages
  each built binary into its platform package, stamps every version to the
  release tag, and publishes the platform packages, the `@velo-ai/ctl`
  meta-package, and the `@velo-ai/omp` plugin in dependency order. Requires an
  `NPM_TOKEN` secret with publish rights on the `@velo-ai` org.

## Included since 0.0.5 — self-update & Claude skill parity

- **`ctl update` — in-place self-updater.** A new top-level command resolves the
  latest release from the `neostfox/ctl` GitHub repo, downloads the platform
  asset over HTTPS, **sha256-verifies** it against the published `.sha256`
  (refusing to install if the checksum is missing), extracts it with the system
  `tar`, and replaces the running binary (Windows renames the live `.exe` aside;
  Unix replaces the inode). `ctl update --check` reports without installing;
  `--version <tag>` pins a specific release. This is the **only** command that
  performs network I/O.
- **ADR 0002 — narrow network carve-out.** `ctl update` deliberately overturns
  the `DEP-002` blanket "no HTTP client" stop with an audited, narrow carve-out:
  one synchronous client (`ureq`, **native-tls** backend — no async runtime, no
  C/asm toolchain on the local Windows build), against a pinned release host,
  sha256-verified, never on the governed task/run/gate path and producing no
  events. `reqwest`/`tokio`/`hyper`/`async-std` stay hard-forbidden by the
  `check_dependencies` guard; the event ledger stays pure and offline. On
  **Linux** the build uses native-tls's **vendored** OpenSSL (compiled from
  source), so the cross-compiled `aarch64` artifact builds without a system
  OpenSSL and every Linux binary is statically self-contained (no end-user
  libssl dependency). macOS (Security.framework) / Windows (schannel) are
  unaffected.
- **Claude skill parity — spec lifecycle.** The `ctl-spec-bootstrap` and
  `ctl-spec-update` skills are now shipped to the Claude adapter
  (`claude_embedded_files()`), closing the two genuine gaps where the capability
  had no Claude path. (The other OMP-native skills remain covered differently on
  Claude by design: `ctl-diagnose` via the `ctl-oracle` subagent,
  `ctl-brainstorm`/`ctl-review` folded into `control-guard`.) `control-guard` now
  routes the spec lifecycle; the spec-bootstrap hook-integration section is
  honest that the Claude `ctl-context.py` is SessionStart-only (no automatic
  spec-drift detection).
- **ADR 0001 — defer cryptographic authentication & signed envelopes.** Records
  the decision to keep authenticated-principal / signed-orchestration-envelope
  work deferred at lowest priority for ctl's local, single-user, trusted-operator
  model — honest disclosure is the sufficient response; crypto would not deliver
  the property locally and needs guardrail-forbidden deps.

## Included since 0.0.4 — record-and-disclose hardening

Deliverables of the orchestration-trust audit
(`brainstorms/orchestration-trust-audit-v1.md`). **None of this is cryptographic
proof** — every new record is *host-attested evidence*, disclosed as such; the
audit's "Do Not Claim" list still holds (no authenticated principal, no signed
envelopes — those need dependencies the guardrail forbids).

- **Honest per-tool/per-platform gate disclosure.** The Claude SessionStart
  message and the boundary sections (here / README / DESIGN) now state the truth:
  `Write`/`Edit`/`MultiEdit` fail **closed** when ctl is unavailable, but Claude
  `Bash` fails **open** and the **`Task` tool is not gated by PreToolUse at all**
  (a Claude platform boundary — U-1 — not a TODO).
- **Gate decision log (non-canonical).** All three host hooks now call
  `ctl hook record-decision` on a deny or a `bash_write` allow, appending to
  `.ctl/decisions.jsonl`; **`ctl decisions`** views it behind a NON-CANONICAL
  banner — advisory evidence, never a task event, not hash-chained, not covered by
  `ctl validate`.
- **Claude hook coverage.** First automated tests for the Claude python hooks
  (per-tool fail-closed/open, the ungoverned `Task` boundary, decision-log
  recording, the honest SessionStart wording). They run in CI (a `claude-hooks`
  job) and under **`ctl adapter doctor --verify`** (`platform.claude_hook_tests`).
  `ctl adapter doctor` also gained a Claude hook-platform check
  (`gate.py`/`context.py`/`settings.json` present + the PreToolUse matcher), with
  no change to Claude's non-adapter status.
- **Runs reach Finished.** **`ctl run finish`** is the production caller
  `run_finished` previously lacked — a run now reaches Completed and drops out of
  recovery instead of looking forever open.
- **Run provenance (host-attested).** `ctl run finish` records optional
  `model`/`provider`/`started_at`/`ended_at`/`exit_code` and the sha256 of supplied
  instruction/context/output artifacts onto the run — recorded by ctl, **not
  verified**.
- **Subagent dispatch attestation (host-attested).** A new canonical
  `subagent_dispatched` task event records role/adapter/parent + artifact hashes
  via **`ctl dispatch record`** (viewable with **`ctl dispatch list`**). OMP and
  OpenCode auto-record an allowed subagent spawn; **Claude cannot** (U-1), by
  design. role/adapter are host labels; ctl records what it was told, not what ran.

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
  `CTL_TASK_ID` binding and routes its tool calls through the gate. The **U-1 spike
  (2026-06-20) resolved** the previously-open host question against us: on Claude
  Code, PreToolUse does **not** match the `Task`/`Agent` tool, and a spawned
  subagent's own `Write`/`Edit`/`Bash` run in an isolated context that does **not**
  inherit the parent gate. So Claude↔OpenCode subagent-gating parity is a **platform
  structural boundary, not a TODO** — keeping writes inline is the correct design,
  not a stopgap. (OpenCode/OMP gate `task` at the session-plugin level; Claude's
  PreToolUse model cannot.) Writable subagent roles on Claude are deferred by design.

## Known limitations / non-claims

These are deliberate boundaries, not TODOs to silently close:

- **No authenticated principal.** "reviewer ≠ implementer" is enforced by `actor`
  **label** only: audits/approvals are recorded under a distinct `CTL_ACTOR`
  (e.g. `ctl-review`), which is a reviewer **role label, not a proven independent
  identity**. Do not read it as "independently/authentically approved".
- **Write boundaries are not OS sandboxing, and fail-closed is per-tool/per-platform
  — not uniform.** They are tool-hook-layer interceptions (OMP / Claude Code /
  OpenCode); a process that does not route through a hook is unconstrained. The
  path-scoped `Write`/`Edit`/`MultiEdit` tools fail **closed** when ctl is
  unavailable. On Claude Code, **`Bash` fails open** on a ctl error/timeout (the
  shell is never locked out, and Bash is not path-scoped — so it is not a hard write
  boundary), and the **`Task`/subagent-spawn tool is not gated by PreToolUse at all**
  (the U-1 platform boundary above). OpenCode gates `task` and fails `Bash` closed.
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
