# AI Dev Control Plane Sticky Guardrails

These rules apply to every turn. Read `ARCHITECTURE_GUARDRAILS.md` before architecture or implementation changes.

## Active Boundary

Current milestone: `M0: boundary protocol freeze`.

Do not implement later-milestone features unless the user explicitly changes the milestone after reviewing the architectural impact.

## Required Behavior

Every new OMP session starts in read-only exploration mode. A request to understand, inspect, review, plan, or explain the project is not permission to implement.

面向用户的围栏状态、审计报告和完成报告默认使用中文。保留 `ALLOW / ASK / STOP`、`REVIEW`、`audit_hold`、命令、路径和机器字段的原始形式。

Only the human may enter implementation mode by running:

```text
/guardrails-implement <capability> | <relative-file-or-directory/**>, ...
```

`REVIEW` boundaries require a separate exact approval:

```text
/guardrails-review-approve <capability> | Cargo.toml
/guardrails-review-approve <capability> | schemas/**
```

Do not mix ordinary implementation paths and `REVIEW` paths in one approval batch.

Return to read-only exploration mode with:

```text
/guardrails-explore
```

Inspect the active mode and scope with:

```text
/guardrails-status
```

Before editing files in implementation mode:

1. Name the active milestone.
2. Name affected guardrail IDs from `ARCHITECTURE_GUARDRAILS.md`.
3. Freeze the expected implementation scope using the audit contract below.
4. Ask for approval before changing a `REVIEW` boundary.
5. Stop immediately if a `STOP` rule would be violated.

Writes outside the human-declared implementation scope are blocked. Direct shell file mutation is blocked because it cannot be checked reliably against the declared scope.

OMP shell execution is fail-closed during M0. Allowed templates are:

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

Use built-in read, find, search, and LSP tools for exploration. Read-only audits may run the exact shell templates above as separate tool calls. Shell chaining, pipes, redirection, output paths, alternate manifests, and alternate target directories remain blocked. Unknown tools and MCP tools are blocked by default.

## Implementation Audit Contract

实现前，使用中文报告：

```text
里程碑：
请求的能力：
预期文件：
受影响的围栏 ID：
新增依赖：
新增顶层模块：
新增 schema：
新增副作用：
必需验证：
延后工作：
```

Treat this declaration as the baseline for the current implementation batch. It is audit evidence, not canonical state.

Run a read-only guardrail audit:

1. Before implementation begins.
2. Before changing schemas, dependencies, top-level modules, architecture layout, or required gates.
3. After each logical implementation batch or after 5 mutating tool calls, whichever comes first.
4. After verification commands complete.
5. Before claiming a milestone, task, or report is complete.

Each audit trigger enters mandatory read-only `audit_hold`. Writes, scope changes, and additional `REVIEW` approvals are blocked until the human chooses:

```text
/guardrails-resume
/guardrails-explore
```

Each audit must compare expected and observed changes. Do not continue automatically after a `STOP` verdict. For an `ASK` verdict, explain the deviation before continuing. Never ask the human to run a blocked shell command as a bypass.

Audit these deviation signals explicitly:

```text
写入预期文件之外的路径
新增文件或顶层目录
schema 变更
Cargo.toml 或 Cargo.lock 变更
新增 crate 或 runtime dependency
新增 shell、Git、网络、进程或文件系统副作用
schema 已声明事件但 reducer 未处理
reducer 缺少 seq、command_id 或 task_id 不变量
TaskView 字段无法通过 reducer 重建
路径 normalizer 未使用 root 或缺少逃逸检查
协议变更后 fixture 未更新
验证失败后仍声明完成
```

## Hard Rules

- `events.jsonl` is the only canonical truth.
- `task.json` and `control.json` are generated projections.
- telemetry and agent output are evidence, not truth.
- external tools must never append canonical events directly.
- keep `domain/` deterministic and side-effect free.
- keep OMP-specific logic outside `domain/` and `application/`.
- default to `manual` adapter first.
- do not add automatic agent execution before `M4`.
- do not add multi-agent scheduling before `M6`.
- do not use shell network downloads, dependency installation, commit, push, merge, deploy, database migration, or destructive file commands without explicit user approval.
- do not modify `.git/**`, `.trellis/control/**`, canonical `events.jsonl`, this `.omp/` policy directory, `AGENTS.md`, or `ARCHITECTURE_GUARDRAILS.md` from an OMP tool call.
- treat OMP RPC and ACP host execution routes as read-only until the control layer adds a dedicated wrapper; the bootstrap `tool_call` hook does not guard direct host bash execution.

## Bootstrap Review Boundaries

Ask before editing:

```text
Cargo.toml
Cargo.lock
schemas/**
new top-level modules
required gates
architecture layout
```

每次完成变更后，使用中文报告：

```text
里程碑
已检查的围栏 ID
预期文件
实际变更文件
依赖变更
schema 变更
新增副作用
验证命令和结果
剩余偏差、漂移或例外请求
```
