# AI Dev Control Plane 架构围栏

## 目的

本文档定义项目的强制架构边界。后续设计、代码、依赖和自动化都必须先满足这些围栏，再进入实现。

围栏优先于功能数量、开发速度和模型能力。

```text
可表达 -> 可校验 -> 可审计 -> 可隔离 -> 可执行 -> 可并发
```

不能跳级。

## 适用范围

本文档首先约束 `M0-M3`：

```text
M0 边界协议冻结
M1 本地任务账本
M2 验收、边界与归档
M3 Manual 闭环 MVP
```

`M4-M6` 只能在真实 dogfood 达到晋级条件后扩展。扩展时可以新增围栏，不能静默放宽现有围栏。

## 产品围栏

| ID | 级别 | 规则 |
|---|---|---|
| `PRODUCT-001` | STOP | 本项目是本地优先、确定性的 AI 开发控制层，不是模型执行器，不是远程编排平台。 |
| `PRODUCT-002` | STOP | `M3` 之前不接入自动 agent 执行。第一个正式 adapter 必须是 `manual`。 |
| `PRODUCT-003` | STOP | `M3` 之前不引入 daemon、数据库、Web UI、远程服务、Temporal、Vault、策略 runtime。 |
| `PRODUCT-004` | REVIEW | 新能力必须声明所属里程碑、用户价值、机器验收和明确 defer 内容。 |
| `PRODUCT-005` | REVIEW | `small / medium / large` 是风险控制级别，不只是文档模板。系统只能建议升级，不能静默降级。 |

## 事实源围栏

| ID | 级别 | 规则 |
|---|---|---|
| `STATE-001` | STOP | `events.jsonl` 是唯一 append-only canonical truth。 |
| `STATE-002` | STOP | `task.json` 只能由 replay 生成，`control.json` 只能由 reconcile 生成。两者都是可删除投影。 |
| `STATE-003` | STOP | `telemetry.jsonl`、agent 输出和人工回填都是 evidence，不直接修改领域状态。 |
| `STATE-004` | STOP | 外部执行器不能直接追加 canonical event。外部只能提交 evidence，由控制层验证后生成事件。 |
| `STATE-005` | STOP | Markdown 只做人类可读解释，不参与状态判断。 |
| `STATE-006` | REVIEW | MVP 只允许一个 aggregate：`Task`。只有并发执行和独立恢复出现后，才能拆出 `AgentRun` aggregate。 |

事件必须满足：

```text
严格递增 seq
幂等 command_id
严格 schema
未知事件默认拒绝 replay
reducer 不访问时间、Git、文件系统、进程或网络
```

## 状态机围栏

允许的 phase：

```text
planning -> ready -> in_progress -> review -> completed
    |          |           |           |
    +----------+-----------+-----------+-> cancelled
```

`hold` 与 `archived` 必须独立于 phase：

- `hold` 表达越界、等待审批、gate 失败或人工暂停。
- `archived` 表达终态任务的存储位置。
- `boundary_violation_recorded` 必须同时记录越界并进入 hold。

| ID | 级别 | 规则 |
|---|---|---|
| `STATE-010` | STOP | 没有合法 objective、scope 和 required gate 的任务不能进入 `ready`。 |
| `STATE-011` | STOP | 有 hold 的任务不能 `start`、`submit` 或 `finish`。 |
| `STATE-012` | STOP | `finish` 只能发生在 `review`，且所有 required gate 最新结果通过。 |
| `STATE-013` | STOP | 只有 `completed` 或 `cancelled` 任务可以归档。 |
| `STATE-014` | REVIEW | MVP 中任务进入 `ready` 后不能扩大 scope。后续只能通过结构化审批新增 boundary revision。 |

## 模块围栏

`M0-M3` 使用单一 Rust workspace 和一个 CLI 二进制，先通过模块边界控制复杂度。只有出现独立发布、独立恢复或编译隔离需求后，才拆 crate。

推荐布局：

```text
src/
  main.rs
  cli/
  application/
  domain/
  infrastructure/
    store/
    boundary/
    gates/
  adapters/
    manual/
schemas/
fixtures/
tests/
```

依赖方向：

```text
cli -> application -> domain
infrastructure/* -> domain
adapters/manual -> application DTO
main.rs = composition root
```

| ID | 级别 | 规则 |
|---|---|---|
| `MODULE-001` | STOP | `domain/` 不允许依赖 `cli/`、`infrastructure/` 或 `adapters/`。 |
| `MODULE-002` | STOP | `domain/` 不允许直接访问文件系统、Git、进程、网络或当前时间。 |
| `MODULE-003` | STOP | 文件写入、Git diff、命令执行必须分别留在 `store/`、`boundary/`、`gates/`。 |
| `MODULE-004` | STOP | OMP、Codex、Claude、OpenCode 的专有逻辑不能进入 `domain/` 或 `application/`。 |
| `MODULE-005` | REVIEW | 新增顶层模块或拆分 crate 必须说明边界收益和迁移成本。 |

> **强制现状（2026-06-13 收敛后）**：`MODULE-001` / `MODULE-002` 已由 `ctl architecture check` 的 `check_modules` **机器强制**——扫描 `src/domain/**` 模块顶层（列 0）行，禁止 `use crate::{infrastructure,cli,adapters,application}` 与 `use std::{fs,io,net,process,time}` 及 `SystemTime`/`Instant`。`#[cfg(test)]` 测试模块（缩进代码）按约定豁免。此前 `check_modules` 仅校验文件扩展名，规则形同虚设。

## 文件与路径围栏

控制层必须先 canonicalize，再授权，再执行，执行后再次检查实际 diff。

需要处理：

```text
Windows 大小写
反斜杠与正斜杠
绝对路径
..
symlink
junction
UNC
仓库外路径
```

默认受保护路径：

```text
.git/**
.ctl/control/**
.ctl/tasks/**/events.jsonl
schemas/**
Cargo.toml
Cargo.lock
```

| ID | 级别 | 规则 |
|---|---|---|
| `PATH-001` | STOP | 未 canonicalize 的路径不能进入 policy decision。 |
| `PATH-002` | STOP | root 外路径、`..`、绝对路径、symlink、junction 和 UNC 默认拒绝。 |
| `PATH-003` | STOP | agent 不能写受保护路径。依赖文件变化必须触发结构化审批。 |
| `PATH-004` | STOP | assignment 外的实际 diff 必须记录 `boundary_violation_recorded` 并进入 hold。 |
| `PATH-005` | REVIEW | `context build` 默认排除 `.env`、密钥、token、凭据缓存和用户目录。 |

## 执行围栏

| ID | 级别 | 规则 |
|---|---|---|
| `EXEC-001` | STOP | 默认拒绝任意 shell。gate 只能使用允许的命令模板。 |
| `EXEC-002` | STOP | gate runner 必须声明工作目录、环境变量 allowlist、超时、输出上限和日志脱敏。 |
| `EXEC-003` | STOP | 默认禁止网络访问、依赖自动安装和长期密钥注入。 |
| `EXEC-004` | STOP | 默认禁止自动 commit、push、merge、deploy、数据库迁移和安全策略修改。 |
| `EXEC-005` | STOP | telemetry 和 agent 自述不能代替控制层独立执行 gate。 |
| `EXEC-006` | REVIEW | 引入新的副作用类型前，必须先定义 policy、sandbox enforcement 和审计事件。 |

## 审计与完成围栏

任务 phase 不足以表达一次受控执行的状态。控制层必须额外记录执行协议状态：

```text
proposal
  -> pending_approval
  -> scoped_lease
  -> implement
  -> audit_hold
  -> human_resume | completed | stopped
```

- `proposal` 是结构化变更合同，不是一句自然语言授权。
- `scoped_lease` 绑定 task、run、资源、动作、TTL 与最大使用次数。
- `audit_hold` 是强制只读状态。只允许读取、解释和执行确定性的离线 gate。
- `ASK`、`STOP` 与 `UNVERIFIED` 都是合法结果。它们不能被当作“尚未完成”而自动重试。
- reviewer agent 只能提交 evidence。它不能自行批准 lease，也不能替代确定性 gate 或完成闸门。

| ID | 级别 | 规则 |
|---|---|---|
| `AUDIT-001` | STOP | 写入前必须存在已批准的结构化 proposal 和未过期 scoped lease。自然语言确认、模型推断和 capability 声明都不能授予权限。 |
| `AUDIT-002` | STOP | 命中 schema、依赖、scope、required gate、受保护路径或批量变更审计触发器时，必须立即进入 `audit_hold`。 |
| `AUDIT-003` | STOP | `audit_hold` 中禁止继续写入、扩大 scope 或变更 gate。只允许只读检查和允许模板内的离线 gate。 |
| `AUDIT-004` | STOP | 模型不能自行宣布里程碑或任务完成。只有 completion interlock 验证固定证据清单后，才能生成完成事件。 |
| `AUDIT-005` | STOP | baseline manifest 中测试、fixture、schema 或 required gate 数量下降时，默认视为回退并停止。解除必须经过结构化人工审批。 |
| `AUDIT-006` | STOP | 实现者、reviewer 和 telemetry 的 PASS 都只是 evidence。控制层必须独立执行审计矩阵并保留结果。 |
| `AUDIT-007` | REVIEW | 修改 baseline manifest、审计矩阵、完成闸门或审计触发器必须单独审查，不能由实现任务顺带修改。 |
| `AUDIT-008` | REVIEW | 自动 reviewer 接入只能在 adapter 协议和 evidence ingest 已冻结后进行。自动 reviewer 不能成为唯一验收来源。 |

### 协议冻结表

M0 文档层必须冻结以下协议对象。后续写入 `schemas/**` 时必须单独 REVIEW；在 schema 落地前，本文档是设计基线。

| 对象 | 最小字段 | 规则 |
|---|---|---|
| `proposal` | `proposal_id`, `task_id`, `objective`, `milestone`, `requested_scope`, `forbidden_changes`, `expected_schema_changes`, `expected_dependency_changes`, `required_gates`, `risk_triggers` | 自然语言授权不能替代 proposal。 |
| `approval` | `approval_id`, `proposal_id`, `approver`, `decision`, `approved_scope`, `expires_at`, `reason` | 只能批准 proposal 中明确列出的资源和动作。 |
| `scoped_lease` | `lease_id`, `task_id`, `run_id`, `subject`, `actions`, `resources`, `issued_at`, `expires_at`, `max_uses`, `revoked_at` | 写入前必须存在未过期、未撤销、未耗尽的 lease。 |
| `assignment` | `assignment_id`, `task_id`, `adapter`, `contract`, `scopes`, `context_hashes`, `required_capabilities`, `acceptance`, `lease_id` | adapter 只能消费 assignment，不能自行推断 scope。 |
| `evidence` | `evidence_id`, `run_id`, `source`, `command`, `exit_code`, `touched_files`, `input_hashes`, `output_hashes`, `artifact_hashes`, `self_report`, `trust_level` | evidence 不是真相；必须经控制层验证。 |
| `audit_report` | `audit_id`, `task_id`, `trigger`, `observed_files`, `dependency_changes`, `schema_changes`, `gate_results`, `baseline_result`, `verdict`, `rule_ids` | audit report 可以解释判定，不能绕过 interlock。 |
| `completion_interlock` | `task_id`, `required_evidence`, `satisfied_evidence`, `pending_approvals`, `baseline_status`, `gate_status`, `verdict` | 只有 `verdict=allow` 才能生成完成事件。 |
| `drift_report` | `task_id`, `evidence_ids`, `signals`, `score`, `action`, `explanation`, `generated_proposal_id` | drift 只能生成暂停、说明或新 proposal，不能自动扩权。 |

### Canonical event 类别

完整事件表在 schema 冻结时落地；设计层先固定事件类别：

| 类别 | 示例 | 说明 |
|---|---|---|
| Task lifecycle | `task_created`, `task_marked_ready`, `task_started`, `task_submitted_for_review`, `task_completed`, `task_cancelled`, `task_archived` | 改变 Task phase 或 archived 属性。 |
| Boundary / hold | `boundary_violation_recorded`, `hold_entered`, `hold_exited` | 表达越界、暂停和解除；hold 不是 phase。 |
| Gate / evidence verified | `gate_checked`, `evidence_accepted`, `evidence_rejected` | 记录控制层验证后的 evidence 结论。 |
| Approval / lease | `approval_requested`, `approval_granted`, `approval_denied`, `lease_issued`, `lease_revoked`, `lease_expired` | 表达授权生命周期。 |
| Audit / completion | `audit_report_recorded`, `completion_interlock_passed`, `completion_interlock_failed` | 记录完成闸门的机器判定。 |
| Drift / replan | `drift_updated`, `replan_proposed`, `rescope_requested` | 只能提出新控制动作或 proposal，不能直接扩权。 |

### `audit_hold` 触发器

以下信号必须立即进入 `audit_hold`：

```text
schema 变化
Cargo.toml 或 Cargo.lock 变化
新增 runtime dependency
新增顶层模块或 crate
scope 扩大或实际 diff 越界
受保护路径变更
required gate 变化
baseline manifest 回退
gate 失败或无法执行
completion interlock evidence 缺失
新增 shell、Git、网络、进程或文件系统副作用类型
引入当前 milestone 之外的能力
```

解除 `audit_hold` 只能通过控制层记录结构化结果：

```text
human_resume      = 人类确认继续，但不扩大 scope
replan_proposed   = 当前 lease 停止，生成新的 proposal
stopped           = 当前执行终止
completed         = completion interlock 通过后生成完成事件
```

`audit_hold` 中禁止新增 approval、扩大 scope、修改 gate 或继续写入。

### Completion interlock 固定证据清单

完成事件前必须独立检查：

```text
1. Task phase 是 review，且没有 hold
2. 所有 required gate 最新结果为 PASS
3. 实际 touched_files 全部落在 approved scope 内
4. 无 pending approval、过期 lease、已撤销 lease、已耗尽 lease、跨 task lease 或重复 lease
5. baseline manifest 未回退
6. schema、依赖、受保护路径、required gate 变化均已按 REVIEW/approval 处理
7. evidence hash、input_hashes、output_hashes 可解析且与报告一致
8. reviewer / agent / telemetry 的 PASS 未被当作唯一验收来源
```


## Adapter 围栏

| ID | 级别 | 规则 |
|---|---|---|
| `ADAPTER-001` | STOP | `manual` 是第一个正式 adapter，先验证 assignment 协议。 |
| `ADAPTER-002` | STOP | adapter 只能消费版本化 assignment，提交版本化 evidence。 |
| `ADAPTER-003` | STOP | adapter capability 声明不是授权证明。所有动作仍需通过 policy。 |
| `ADAPTER-004` | STOP | `M4` 才允许 OMP 单执行器，并且只能写 disposable worktree。 |
| `ADAPTER-005` | STOP | `M6` 前禁止多个 agent 并发写入；`M6` 后重叠写 scope 仍然拒绝。 |
| `ADAPTER-006` | REVIEW | 新 adapter 必须通过与 `manual` 相同的 contract tests。 |
| `ADAPTER-007` | STOP | 在控制层提供专用 wrapper 前，OMP RPC / ACP host execution 只能用于只读检查，不能作为受围栏保护的执行入口。 |

M6 前不得把 `AgentRun` 拆成独立 aggregate。M3/M4 中的 run 只是 `Task` aggregate 下的 evidence source；adapter output 永远不能直接追加 canonical event，必须经 evidence ingest、scope check、gate check 和 interlock 验证。


## 依赖围栏

M0 只允许引入完成 schema、CLI 骨架和测试所需的最小依赖。每个新增 runtime dependency 都必须解释用途、替代方案和退出路径。

初始允许评估的依赖类别：

```text
CLI parsing
serde JSON
error handling
JSON Schema validation
hash
glob matching
file locking
test fixtures
```

| ID | 级别 | 规则 |
|---|---|---|
| `DEP-001` | REVIEW | 新增 runtime dependency 必须单独审查。 |
| `DEP-002` | STOP | `M0-M3` 禁止引入 async runtime、HTTP client、数据库、Web framework、agent SDK。 |
| `DEP-003` | STOP | 禁止自动执行依赖安装或 package script。 |
| `DEP-004` | REVIEW | `Cargo.toml` 和 `Cargo.lock` 变化必须进入依赖变更报告。 |

## 漂移计算

硬围栏优先于分数。任何 `STOP` 规则命中时：

```text
记录 architecture_guardrail_violated
进入 hold
停止自动执行
请求结构化审批或重新规划
```

只有未命中硬围栏时，才计算软漂移：

| 信号 | 分数 |
|---|---:|
| 新增未计划 runtime dependency | `0.30` |
| 新增顶层模块或 crate | `0.20` |
| schema 变化 | `0.40` |
| scope 扩大 | `0.20` |
| 新增副作用类型 | `0.40` |
| 引入里程碑之外的能力 | `0.40` |
| required gate 变化 | `0.30` |

分数累加并截断到 `1.0`：

| Drift | 动作 |
|---|---|
| `< 0.2` | continue |
| `0.2 - 0.4` | annotate next task |
| `0.4 - 0.6` | replan remaining tasks |
| `> 0.6` | stop and ask human |

drift 升高只能触发暂停、解释或重规划，不能自动扩大权限。

SDD 风格的 `effort_delta`、`super_delta`、`unplanned_deps` 可以作为 telemetry evidence 进入 drift report，但不能替代硬围栏，也不能自动生成 approval。它们的唯一作用是解释 `annotate`、`replan_proposed` 或 `rescope_requested` 为什么发生。


## 例外协议

围栏例外必须是结构化记录，不能只写一句自然语言。

```json
{
  "schema": "control.architecture-exception.v1",
  "rule_id": "DEP-002",
  "task_id": "05-30-example",
  "requested_by": "shaob",
  "reason": "why this exception is necessary",
  "scope": ["exact resource or action"],
  "compensating_gates": ["additional checks"],
  "approved_by": "human-id",
  "expires_at": "2026-06-01T00:00:00Z"
}
```

硬约束：

- 例外只能缩小到明确任务、资源和动作。
- 例外必须有过期时间。
- 例外不能覆盖 `STATE-001`、`STATE-004`、`EXEC-005`。
- drift 升高不能自动生成或批准例外。

## 每次变更必须回答

```text
属于哪个里程碑？
改变了哪条边界？
是否新增依赖、模块、schema 或副作用？
是否触碰受保护路径？
实际 diff 是否仍在 scope 内？
哪些 gate 证明它仍然满足围栏？
失败时能否停在可恢复位置？
```

## M0 必须实现的机器检查

```text
ctl schema validate
ctl boundary check
ctl boundary explain
ctl architecture check
```

M0 必须冻结 baseline manifest。它至少记录：

```text
schema 集合
fixture 集合
required gate 集合
审计矩阵版本
测试用例数量或等价的稳定检查项
```

`ctl architecture check` 至少要检查：

- schema 未知字段。
- 非法状态转换。
- 路径逃逸。
- 受保护路径变更。
- 新增依赖。
- 顶层模块变化。
- milestone 外能力。
- required gate 变化。
- baseline manifest 回退。

M0 固定审计矩阵至少覆盖：

```text
Schema 正例、未知字段、缺失字段、格式错误
reducer 合法转换、非法转换、hold 阻断、重复 command_id、乱序 seq
相同事件流 replay 字节级一致
Windows 大小写、分隔符、绝对路径、..、symlink、junction、UNC、root escape
受保护文件变更
依赖白名单与禁止依赖
required gate 变化
baseline manifest 回退
```

机器审计失败、证据缺失或无法执行时，只能输出：

```text
STOP
ASK
UNVERIFIED
```

不得降级为实现者自述的 PASS。

## 已冻结决策

```text
local-first CLI
Rust 实现
边界优先
events.jsonl 唯一 canonical truth
task.json / control.json 是投影
telemetry 是 evidence
MVP 只有 Task aggregate
manual-first
M3 前无自动 agent
M3 前无网络、数据库、daemon、Web UI
```

## M0 仍需决策

1. 项目正式名称、crate 名称和 CLI 名称。
2. task ID 与 event ID 的格式。
3. Windows 路径规范化的精确算法。
4. JSONL append 的文件锁、checksum、`fsync` 与崩溃恢复策略。
5. gate 命令模板格式与默认 allowlist。
6. `.ctl/` 兼容路径是否作为长期 canonical layout。
7. 架构例外记录进入 task event，还是独立 policy journal。
8. proposal、scoped lease、audit report 与 baseline manifest 的精确 schema。
9. completion interlock 的固定证据清单和解除 `audit_hold` 的事件协议。
