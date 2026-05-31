# OMP Project Integration

This directory contains project-scoped OMP guardrail assets.

## Loaded Assets

```text
RULES.md
  Sticky always-apply project rules. OMP re-injects these near the current turn.

settings.json
  Project overrides and explicit extension loading. `tools.approvalMode` intentionally remains `yolo`; audit and blocking rules are separate controls.

skills/control-plane-guardrails/SKILL.md
  On-demand architecture audit and implementation workflow.

commands/guardrails.md
  Interactive `/guardrails` audit command.

agents/architecture-reviewer.md
  Read-only subagent for drift review.

hooks/pre/guardrails.ts
  Bootstrap blocking hook for protected paths, path escape attempts, and clearly dangerous shell commands.
```

## Supported Development Mode

Use normal interactive OMP or ordinary print-mode agent runs from the repository root:

```text
omp
omp -p "..."
```

The project configuration is discovered from `.omp/`.

After changing the TypeScript guardrail extension, start a new OMP process. An already running process keeps its previously loaded extension module.

## Runtime Modes

Every OMP session starts in read-only exploration mode. This prevents a request such as "understand the project" from silently turning into implementation.

Enter scoped implementation mode explicitly:

```text
/guardrails-implement implement M0 reducer invariants | src/domain/**, fixtures/**
```

Approve a `REVIEW` boundary as a separate exact batch:

```text
/guardrails-review-approve evaluate schema contract | schemas/**
/guardrails-review-approve evaluate dependency change | Cargo.toml
```

Return to read-only mode or inspect the active contract:

```text
/guardrails-explore
/guardrails-status
```

In implementation mode, `write`, `edit`, and `ast_edit` are limited to the declared relative paths. Direct shell file mutation is blocked because it bypasses reliable scope checks.

Before every agent run, the extension injects the active guardrail mode, capability, and allowed scopes into model context. Slash-command mode changes therefore remain visible to the next model turn instead of relying on the model to infer state from prior tool results.

Tool execution is fail-closed during M0. Exploration uses built-in read, find, search, and LSP tools. Read-only audits may also use the same strict OMP shell allowlist as implementation mode:

```text
cargo fmt --check
cargo check --locked --offline
cargo test --locked --offline
cargo clippy --locked --offline -- -D warnings
cargo run --locked --offline -- architecture check
git status --short
git diff
git diff --stat
git diff --name-only
git diff --name-status
git diff --check
```

Run each shell template exactly as shown and as a separate tool call. Shell chaining such as `&&`, pipes, redirection, output paths, alternate manifests, and alternate target directories remain blocked. `--locked` is mandatory because Cargo dependency resolution can modify `Cargo.lock` even when `--offline` prevents network access.

Unknown tools and MCP tools are blocked by default. This prevents an enabled global tool from silently expanding the project execution surface.

## Security Boundary

The TypeScript pre-hook blocks ordinary agent `tool_call` events.

Do not treat OMP RPC or ACP host execution routes as covered by this bootstrap hook. OMP RPC exposes a direct host `bash` command that bypasses the agent `tool_call` path. Until the control layer implements a dedicated wrapper, use RPC/ACP for read-only inspection only.

This hook is defense in depth, not a sandbox. The future control layer remains responsible for canonical path checks, policy decisions, capability leases, diff verification, and sandbox enforcement.

## Audit Workflow

The OMP bootstrap is optimized for scoped implementation with mandatory audit pauses:

```text
declare expected implementation scope
implement one logical batch
enter audit_hold
run a read-only guardrail audit
compare expected and observed changes
human /guardrails-resume or /guardrails-explore
```

Schemas and dependency files cannot enter an ordinary implementation contract. They require a separate `/guardrails-review-approve` batch. After the approved mutation, the hook immediately enters `audit_hold`. It also enters `audit_hold` after 5 successful mutating tool calls.

`audit_hold` is read-only. Writes, scope changes, and additional review approvals remain blocked until a human runs `/guardrails-resume` or returns to exploration with `/guardrails-explore`. The trigger is persisted as an OMP session entry with `canonical: false`; it is evidence only.

The audit must report file-line evidence for deviations and distinguish:

```text
STOP   hard guardrail violation; stop implementation
ASK    review boundary or unexplained deviation; explain before continuing
ALLOW  no hard violation; continue with any recorded drift
```

面向用户的围栏状态、审计报告和完成报告默认使用中文。机器关键字 `STOP / ASK / ALLOW`、`REVIEW`、`audit_hold`、命令、路径和机器字段保持原样。

When Git metadata is unavailable, the implementation batch must report its touched files explicitly. OMP reports are evidence only; they never become canonical events directly.

## Manual Verification

From the repository root:

```text
/guardrails
/skill:control-plane-guardrails
bun test ./.omp/tests/guardrails.test.ts
```

For a session-level discovery probe without invoking a model, start OMP RPC mode and issue `get_state`. The system prompt should include:

```text
AI Dev Control Plane Sticky Guardrails
AI Dev Control Plane Development Context
control-plane-guardrails
```
