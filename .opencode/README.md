# opencode integration for ctl

This directory wires the **ctl control plane** into an [opencode](https://opencode.ai)
session, mirroring the `.claude/` (Claude Code) and `.omp/` (OMP) integrations.
The governance logic lives in the `ctl` binary; the files here are thin
opencode-side shims.

## Contents

| Path | Role |
|---|---|
| `plugins/ctl-gate.ts` | opencode plugin: gates `write`/`edit`/`patch`/`bash`/`task` via `ctl hook gate` and injects active task boundaries via `ctl hook context`. |
| `skills/control-guard/SKILL.md` | Entry-point skill: proactive task lifecycle routing, the subagent-role dispatch table, and the close-out (audit → finish → archive) protocol. |
| `agent/designer.md`, `agent/oracle.md` | Custom subagent roles the primary agent dispatches by phase (design → `designer`, diagnosis → `oracle`). Mirrors the `.omp` role set under opencode-native names; built-in `explore` (read-only) and `build` (implementation) cover the rest. The `task`-tool gate governs each spawn. |

## How enforcement works

- **`tool.execute.before`** queries `ctl hook gate` and **throws** (aborting the
  tool call) on an out-of-scope or wrong-phase verdict.
- **`experimental.chat.system.transform`** appends the active task's write scope,
  deny list, and gates to the system prompt every turn.
- **Fail closed**: if `ctl` is missing, times out, or returns unparseable output,
  mutating tools (`write`/`edit`/`patch`/`bash`/`task`) are **blocked** until ctl
  responds. Read-only tools are never blocked on ctl errors.

## Tests

The plugin contract is verified with Bun (`.opencode` is a dot-dir, so pass it as cwd):

```bash
bun test --cwd .opencode
```

These cover tool classification, gate-arg construction (array form — no shell
splitting), allow/deny/fail-closed behavior, and context injection — over an
injected ctl runner, plus one test through the real exported hook. They run as a
required CI job (`opencode-plugin`). The shared **Rust** adapter conformance
suite (`cargo test`, in `src/adapters`) covers the executor-adapter side for
every adapter. Live model-driven E2E is a pre-release dogfood step, not CI.

## Requirements

- `ctl` must be on `PATH` (`ctl doctor` to verify).
- opencode auto-loads plugins from `.opencode/plugins/*.{ts,js}`. Current opencode
  scans the brace glob `{plugin,plugins}`, so the singular `.opencode/plugin/`
  also works; the plural form here follows the official docs.

## Optional environment seams

| Variable | Effect |
|---|---|
| `CTL_TASK_ID` | Bind the gate to a specific task (resolves multi-active ambiguity). |
| `CTL_TIMEOUT_MS` | Override the per-call `ctl` timeout (default 15000 ms). |

## Adapter (run ingestion)

For autonomous runs, `ctl` also ships an `opencode` **executor adapter**
(`ctl adapter capabilities --adapter opencode`). An agent-output file ingested
with `--adapter opencode` is validated to carry `source: "opencode"` and a
`touched_files` array, then scope-checked against the task's `write_allow`:

```bash
ctl run ingest --id <task> --adapter opencode --result <agent-output.json>
```
