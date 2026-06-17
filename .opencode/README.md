# opencode integration for ctl

This directory wires the **ctl control plane** into an [opencode](https://opencode.ai)
session, mirroring the `.claude/` (Claude Code) and `.omp/` (OMP) integrations.
The governance logic lives in the `ctl` binary; the files here are thin
opencode-side shims.

## Contents

| Path | Role |
|---|---|
| `plugins/ctl-gate.ts` | opencode plugin: gates `write`/`edit`/`patch`/`bash`/`task` via `ctl hook gate` and injects active task boundaries via `ctl hook context`. |
| `skills/control-guard/SKILL.md` | Entry-point skill: proactive task lifecycle routing and the close-out (audit → finish → archive) protocol. |

## How enforcement works

- **`tool.execute.before`** queries `ctl hook gate` and **throws** (aborting the
  tool call) on an out-of-scope or wrong-phase verdict.
- **`experimental.chat.system.transform`** appends the active task's write scope,
  deny list, and gates to the system prompt every turn.
- **Fail closed**: if `ctl` is missing, times out, or returns unparseable output,
  mutating tools (`write`/`edit`/`patch`/`bash`/`task`) are **blocked** until ctl
  responds. Read-only tools are never blocked on ctl errors.

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
