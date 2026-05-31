---
name: control-plane-guardrails
description: Apply the AI Dev Control Plane architecture guardrails before planning, editing, reviewing, or adding dependencies. Use for all Rust control-layer work, schema changes, module changes, gate design, and architecture audits.
---

# Control Plane Guardrails

Use this workflow before changing the AI Dev Control Plane project.

面向用户的围栏状态、审计报告和完成报告默认使用中文。保留 `ALLOW / ASK / STOP`、`REVIEW`、`audit_hold`、命令、路径和机器字段的原始形式。

## Read First

Read:

```text
ARCHITECTURE_GUARDRAILS.md
ROADMAP.md
AGENTS.md
```

Treat `.omp/RULES.md` as sticky mandatory rules.

## Runtime Mode

Every new OMP session starts in read-only exploration mode. Understanding, inspection, review, planning, and explanation requests do not authorize implementation.

Only the human may enable writes with:

```text
/guardrails-implement <capability> | <relative-file-or-directory/**>, ...
```

`REVIEW` boundaries such as `Cargo.toml`, `Cargo.lock`, and `schemas/**` require a separate human command:

```text
/guardrails-review-approve <capability> | <review-boundary-path>, ...
```

Use `/guardrails-resume` after reviewing an `audit_hold`, `/guardrails-explore` to return to read-only mode, and `/guardrails-status` to inspect the current contract.

Writes outside the declared scope and direct shell file mutations are blocked by the project hook.

OMP shell execution is fail-closed during M0. Use built-in read, find, search, and LSP tools for exploration. Only the exact Cargo verification templates listed below, `cargo run --locked --offline -- architecture check`, `git status --short`, and the read-only `git diff` templates are allowed. Output paths, alternate manifests, and alternate target directories are blocked.

## Classify The Change

使用中文报告：

```text
当前里程碑
请求的能力
受影响的围栏 ID
预期文件
新增依赖
新增顶层模块
新增 schema
新增副作用
必需验证
延后工作
```

Freeze this as the implementation audit contract before editing. The contract is audit evidence, not canonical state.

Use these outcomes:

```text
ALLOW   = within current milestone and no REVIEW or STOP rule is touched
ASK     = touches a REVIEW boundary or needs a structured exception
STOP    = violates a STOP rule
```

Do not silently broaden scope.

## M0 Allowed Work

```text
Rust CLI skeleton
JSON Schema definitions
domain event envelope
Task phase and transition rules
pure reducer fixtures
Windows path normalization design and tests
architecture check design
```

Do not implement adapters, telemetry scoring, scheduling, network services, databases, daemon processes, or Web UI during M0.

## Architecture Audit

Check:

1. Is `domain/` still pure?
2. Are state changes represented as canonical events?
3. Are projections rebuildable?
4. Is telemetry treated only as evidence?
5. Did a new dependency, module, schema, gate, or side effect appear?
6. Did the change touch a protected path?
7. Is the work still inside the active milestone?
8. Can failure stop at a recoverable point?
9. Do observed touched files remain inside the expected file list?
10. Does each event declared in schema have reducer handling?
11. Does replay enforce `seq`, `command_id`, and `task_id` invariants?
12. Can every required TaskView field be reconstructed by replay?
13. Does path normalization enforce root containment and escape rejection?
14. 完成声明是否与实际验证结果一致？

Run this read-only audit:

```text
before implementation
before schema, dependency, module, architecture, or required-gate changes
after each logical implementation batch or 5 mutating tool calls
after verification
before claiming completion
```

Every automatic audit trigger enters mandatory `audit_hold`. Do not edit files, change scope, approve another review boundary, or ask the human to run a blocked shell command as a bypass. Report `STOP / ASK / ALLOW` and wait for the human to choose `/guardrails-resume` or `/guardrails-explore`.

使用中文对比：

```text
预期能力和文件
实际修改文件
依赖
顶层模块
schema
副作用
验证命令和结果
协议覆盖缺口
```

When Git metadata is unavailable, state that limitation and use the touched-file list reported by the implementation batch. Do not invent a complete diff.

## Verification

When code exists, run focused verification:

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

最后输出简短的中文围栏报告：

```text
里程碑：
已检查的围栏：
预期文件：
实际变更文件：
依赖变更：
schema 变更：
新增副作用：
验证：
偏差：
未命中 STOP 时的漂移：
例外：
下一步：
```
