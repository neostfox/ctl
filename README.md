# AI Dev Control Plane 方案记录

## 目标

把以下能力融合成一个更干净的体系：

- **OMP**：作为主执行器，负责模型路由、代码编辑、LSP、DAP、浏览器、subagents、工具调用。
- **Trellis**：吸收其任务、归档、spec、workspace journal、context manifest 能力。
- **Spec-Driven Develop**：重点吸收钱学森工程控制论式闭环，而不是照搬整套 Markdown 流程。
- **Rust 重写控制层**：把任务状态、事件、telemetry、drift 计算做成确定性状态机。

一句话定位：

> Trellis 的任务/归档/记忆能力 + Spec-Driven 的闭环控制系统，用 Rust 做成确定性 AI 开发控制层，OMP 作为执行器。

---

## 第一原则：边界优先

边界是第一优先级，高于功能数量、自动化程度和模型能力。

详细落地顺序见：[ROADMAP.md](./ROADMAP.md)。

后续设计与实现必须遵守：[ARCHITECTURE_GUARDRAILS.md](./ARCHITECTURE_GUARDRAILS.md)。

这个项目首先要解决的不是“让 agent 做更多事情”，而是明确：

```text
什么可以做？
什么不能做？
谁可以做？
在什么范围内做？
越界时由谁决定是否继续？
```

所有能力都必须建立在可定义、可检查、可审计的边界之内。

### 必须守住的边界

| 边界 | 约束 |
|---|---|
| 项目边界 | Rust 控制层负责状态、事件、权限、验收和调度，不负责重新实现模型执行器。 |
| 阶段边界 | 第一阶段只替换 Trellis 的 task 能力；telemetry、drift、agent schedule 分阶段加入。 |
| 任务边界 | 每个任务必须声明目标、范围、允许修改的文件和验收 gate。 |
| 写入边界 | 子智能体默认最小权限；只有明确授权后才能写代码、改依赖或操作 Git。 |
| 文件边界 | agent 只能修改 assignment 中允许的路径；越界修改必须停止并记录事件。 |
| 架构边界 | 公共 API、依赖、数据库迁移和安全策略变化必须触发人类确认。 |
| Adapter 边界 | 控制层只依赖统一协议，不绑定 OMP、Codex、Claude 或 OpenCode 的内部实现。 |
| 自动化边界 | 无法解释的 drift、权限不足和高风险动作必须暂停，不能自动扩大权限继续执行。 |

衡量一个功能是否应进入系统，先问：

```text
它的边界能否被机器表达？
它的越界能否被检测？
它的行为能否被事件日志解释？
失败时能否停在一个可恢复的位置？
```

如果答案是否定的，该功能暂时不进入自动执行路径。

### 从 OMP Dogfood 吸收的控制协议

当前项目级 OMP 围栏验证了默认只读、scope 限制、受保护路径和 shell 模板白名单的价值，也暴露了一个更重要的问题：

```text
限制模型能否写入
!=
限制模型能否继续修改、扩大范围或宣布完成
```

因此，正式控制层必须冻结一条独立于执行器和模型的协议：

```text
proposal
  -> pending_approval
  -> scoped lease
  -> implement
  -> audit_hold
  -> deterministic audit
  -> human_resume | completed | stopped
```

硬约束：

- 批准对象是结构化 proposal，不是自然语言中的“继续”或“批准”。
- lease 绑定 task、run、资源、动作、TTL 与最大使用次数。
- 命中 schema、依赖、scope、required gate、受保护路径或批量变更触发器后，立即进入只读 `audit_hold`。
- completion interlock 独立检查固定证据清单。模型和 reviewer 都不能自行宣布里程碑完成。
- baseline manifest 记录 schema、fixture、required gate、审计矩阵和稳定检查项。数量下降时默认 `STOP`，即使现有测试仍然通过。
- `ASK`、`STOP` 和 `UNVERIFIED` 是合法结果，不应被自动重试逻辑覆盖。

M0 现在冻结协议、审计矩阵和机器检查。正式 OMP adapter 与自动 reviewer 仍然留在 M4 阶段及以后；自动 reviewer 只提供 evidence，不取代确定性 gate。

---

## OMP 与 Codex / Claude Code / OpenCode 的判断

### OMP 优势

- 工具面完整：`read/search/find/edit/lsp/debug/browser/task` 统一接口。
- Windows 原生，无需 WSL。
- 多模型路由强：GPT、Claude、GLM、DeepSeek、MiMo、Copilot、Cursor、Kimi 等可按 role 分配。
- LSP / DAP 深度集成：能做引用、重命名、诊断、调试。
- 文件与 URL 抽象统一：本地文件、GitHub PR/Issue、skills、URL、PDF 都可通过 `read` 处理。
- 更适合高可靠工程：hashline edit、subagents、debug、review、LSP 能降低误改概率。

### OMP 弱势

- 生态规模不如 Claude Code / Codex 官方工具。
- 功能密度高，复杂度也高。
- 官方模型一手体验：Claude Code 对 Claude、Codex 对 OpenAI 可能更稳。
- 插件兼容不等于 hook 行为完全等价。
- 团队采纳需要额外培训。

### 结论

个人/小团队高强度工程执行：

```text
OMP = 主力执行器
Trellis-like layer = 项目治理层
Spec-Driven control loop = 大型任务控制论
Claude/Codex/OpenCode = 兼容目标或备用执行器
```

团队推广时保持兼容：

```text
仓库里放 .trellis / .agents/skills / AGENTS.md / .omp/skills
让 OMP、Codex、Claude、OpenCode 都能读。
```

当前仓库已经先落地 OMP 项目级围栏支持，使用说明见：[.omp/README.md](./.omp/README.md)。

---

## 模型分工建议

按失败代价分工，不按厂商平均轮换。

| 工作 | 首选模型 |
|---|---|
| 架构方案 / API 决策 / 迁移计划 | GPT / Codex 高推理 |
| 高风险调试 / 最终 review | GPT / Codex 高推理 |
| 日常实现 | GLM |
| 批量重构 | DeepSeek |
| 单测补齐 | DeepSeek / GLM |
| 简单子任务 / 摘要 / commit | Xiaomi MiMo |
| S.U.P.E.R 架构评估 | GPT + GLM 交叉 |

OMP 推荐配置示例：

```yaml
modelRoles:
  plan:    openai/gpt-5.3-codex:high
  slow:    openai/gpt-5.3-codex:high
  default: z.ai/glm-5
  smol:    xiaomi/<mimo-model-id>
  commit:  xiaomi/<mimo-model-id>
```

DeepSeek 可作为默认实现模型切换：

```yaml
modelRoles:
  default: deepseek/<model-id>
```

---

## Trellis 应吸收的能力

Trellis 不是纯 Python 项目，主体仓库是 TypeScript，但落地到项目里的核心脚本大量使用 Python，例如：

- `.trellis/scripts/task.py`
- `.trellis/scripts/get_context.py`
- `.trellis/scripts/add_session.py`

Rust 重写时，不应照搬实现语言或全部平台适配，而应吸收它的文件协议与生命周期。

### 值得吸收

#### 1. 任务目录

```text
.trellis/tasks/<MM-DD-slug>/
  task.json
  prd.md
  design.md
  implement.md
  implement.jsonl
  check.jsonl
  research/
```

#### 2. 任务生命周期

```text
planning -> in_progress -> review -> completed -> archived
```

#### 3. 归档

```text
.trellis/tasks/archive/YYYY-MM/<task>/
```

#### 4. workspace journal

```text
.trellis/workspace/<developer>/journal-N.md
```

#### 5. spec library

```text
.trellis/spec/
```

#### 6. context manifest

```jsonl
{"file": ".trellis/spec/backend/index.md", "reason": "backend conventions"}
```

### 不建议照搬

- 多平台 generated files 的复杂度。
- 过多 hook 适配。
- 每个平台一套 prompt 的膨胀。
- Python 脚本式状态修改。
- `docs/progress/MASTER.md` 作为第二套进度源。

---

## Spec-Driven Develop 应吸收的能力

最有价值的是它的钱学森工程控制论闭环，不是完整 Phase 0-6 Markdown 流程。

应抽象成真正的控制系统：

```text
Set Point      目标状态：PRD / design / specs / S.U.P.E.R
Plant          被控对象：代码库
Sensor         传感器：diff / tests / LSP / deps / coverage / review
Observer       观测器：把传感器结果转成 telemetry
Comparator     比较器：计算 target vs actual 的偏差
Controller     控制器：决定继续、修正、重规划、暂停询问
Actuator       执行器：OMP / Codex / Claude / OpenCode agent
Feedback       反馈：events.jsonl + telemetry.jsonl + spec updates
```

### 控制动作

| Drift | 动作 |
|---|---|
| `< 0.2` | continue |
| `0.2 - 0.4` | annotate next task |
| `0.4 - 0.6` | replan remaining tasks |
| `> 0.6` | stop and ask human |

硬围栏与 completion interlock 始终先于 drift 分数。命中 STOP、进入 `audit_hold` 或存在 baseline 回退时，不得使用较低 drift 分数继续执行。

### Drift 不应只用百分比

建议计算：

```text
drift_score =
  scope_delta
+ architecture_violation_delta
+ unplanned_dependency_delta
+ test_failure_delta
+ touched_files_delta
+ requirement_coverage_gap
```

示例 telemetry：

```json
{
  "scope_delta": 0.2,
  "super_violations": 2,
  "unplanned_dependencies": ["new-auth-lib"],
  "tests_failed": 3,
  "requirement_coverage_gap": 0.15,
  "drift_score": 0.41,
  "control_action": "replan_remaining_tasks"
}
```

---

## Rust 重写建议

新项目不要叫 Trellis fork，也不要叫 Spec-Driven fork。建议定位为：

```text
AI Development Control System
```

或：

```text
Rust task/control layer for AI-assisted software development.
Trellis-compatible task store, Spec-Driven adaptive control loop.
```

### 推荐模块

```text
src/
  cli.rs
  task_store.rs
  task_state.rs
  archive.rs
  workspace.rs
  spec_store.rs
  context_manifest.rs
  control_loop.rs
  telemetry.rs
  git.rs
  adapters/
    omp.rs
    codex.rs
    claude.rs
    opencode.rs
```

### 文件结构

兼容 Trellis 路径，减少迁移成本：

```text
.trellis/
  spec/
  tasks/
  workspace/
  control/
```

每个任务：

```text
.trellis/tasks/<task>/
  task.json          # 机器状态
  events.jsonl       # 状态事件日志
  telemetry.jsonl    # 执行观测数据
  control.json       # 控制系统当前判断
  prd.md             # 人类可读需求
  design.md          # 技术设计
  implement.md       # 实施计划
  implement.jsonl    # 实现上下文 manifest
  check.jsonl        # 检查上下文 manifest
  research/          # 研究记录
```

### 唯一事实源

必须是：

```text
events.jsonl     = append-only canonical truth
telemetry.jsonl  = append-only evidence index
task.json        = 由 replay 重建的任务投影视图
control.json     = 由 reconcile 重建的决策投影视图
```

而不是 Markdown progress。

外部执行器不能直接追加 canonical event。agent、adapter 和人工工具只能提交 evidence，由控制层验证后生成事件。

Markdown 只做人类可读解释：

```text
prd.md / design.md / implement.md / research/*.md
```

---

## MVP 命令

第一版发布边界是 `M3 Manual 闭环 MVP`，不接模型，不做全平台大一统。详细里程碑见：[ROADMAP.md](./ROADMAP.md)。

```text
control init
control task create
control task ready
control task start
control context build
control task submit
control task finish
control task archive
control task status
control assignment create
control assignment export
control run ingest --adapter manual
control replay
control reconcile
control validate
control doctor
```

### MVP 目标

- 创建任务。
- 记录 PRD/design/implement。
- 生成 implement/check context manifest。
- 由控制层追加 canonical events。
- 检查实际 diff 是否越过 scope。
- 执行 required gates，保存 evidence hash。
- 通过 manual adapter 导出 assignment、回填执行结果。
- replay 状态并生成可审计报告。
- 归档任务并写 journal。

`telemetry / drift / next-action / OMP adapter / schedule` 在 MVP 之后分阶段加入。

---

## 社区可借鉴项目

### GitHub Spec Kit

来源：

- https://github.com/github/spec-kit
- https://martinfowler.com/articles/exploring-gen-ai/sdd-3-tools.html

值得借鉴：

- constitution 概念。
- `/specify -> /plan -> /tasks -> /implement`。
- 模板化 artifacts。
- 多 agent 集成。

注意：容易 Markdown 过载。

可吸收为：

```text
.trellis/spec/constitution.md
```

### Kiro

值得借鉴极简结构：

```text
requirements.md
设计 design.md
任务 tasks.md
```

适合小中型任务。

### Trellis

值得借鉴：

```text
task directory + specs + workspace journal + context manifests
```

### Spec-Driven Develop

只吸收：

```text
S.U.P.E.R
adaptive control loop
drift
large transformation analysis
```

---

## 其他可吸收参考

以下项目值得借鉴，但仍然遵守“边界优先”：第一阶段只吸收协议思想和数据模型，不直接引入新的平台依赖。

### 第一优先级：直接影响核心模型

#### Kubernetes Controller / OpenGitOps

来源：

- https://kubernetes.io/docs/concepts/architecture/controller/
- https://opengitops.dev/

值得吸收：

```text
spec    = desired state
status  = observed state
event   = observation
reconcile(spec, status) -> next action
```

- 控制器不直接假设任务已经成功，而是持续比较目标状态与实际状态。
- 调和动作必须幂等：重复运行不应破坏状态。
- desired state 必须声明式、可版本化、可审计。

落地到本项目：

```text
task.json       = desired task state + materialized status
events.jsonl    = observed facts
control.json    = last reconciliation decision
control reconcile <task>
```

#### Cedar

来源：

- https://docs.cedarpolicy.com/
- https://docs.cedarpolicy.com/auth/authorization.html
- https://docs.cedarpolicy.com/policies/validation.html

Cedar 的授权模型很适合表达 agent 边界：

```text
principal   谁在执行：agent / human / adapter
action      想做什么：read / write / delete / exec / commit / network
resource    对什么做：file / command / dependency / git / secret
context     在什么条件下：task / scope / approval / risk / ttl
```

值得吸收：

- 默认拒绝：没有明确 `permit` 就不能执行。
- `forbid` 优先：命中禁止规则时，即使存在允许规则也拒绝。
- schema 先行：权限请求和策略都应可验证。
- 决策必须返回 diagnostics，说明为什么允许或拒绝。

第一阶段不必立即嵌入 Cedar runtime，但权限数据模型应保持 Cedar-compatible。

#### in-toto / SLSA

来源：

- https://in-toto.io/docs/getting-started/
- https://slsa.dev/spec/v1.2/build-provenance

in-toto 强调：步骤应由授权主体执行，并记录输入材料、输出产物和证据。这个模型可以直接映射到子智能体。

```text
assignment        = layout step
agent             = functionary
allowed files     = artifact rules
input hashes      = materials
output hashes     = products
agent output      = link metadata
```

值得吸收：

- 每次 agent run 记录输入、输出、命令、文件 hash 和执行身份。
- 验收不是只看 agent 自述，还要验证实际产物是否落在授权范围内。
- 后续可为高风险任务增加签名 attestation。

第一阶段先记录 hash 和 evidence，不急着做完整签名链。

### 第二优先级：为后续扩展预留接口

#### Temporal

来源：

- https://docs.temporal.io/

值得吸收：

- durable execution：中断后可以从已记录状态继续。
- replay：从事件历史重建状态。
- 把确定性控制逻辑与有副作用的外部执行分开。
- retry 必须有边界，不能把非幂等动作盲目重放。

本项目只吸收设计原则：

```text
reducer / replay      = deterministic
agent / command run   = side effect
```

第一阶段不引入 Temporal 服务。

#### MCP

来源：

- https://modelcontextprotocol.io/docs/learn/architecture
- https://modelcontextprotocol.io/docs/learn/client-concepts
- https://modelcontextprotocol.io/specification/2025-06-18/client

值得吸收：

- capability negotiation：adapter 先声明能力，再接收任务。
- tools / resources 分离：可执行动作与只读上下文分开。
- tool input 使用 schema。
- roots 用于告诉执行器当前工作范围。

重要限制：

```text
MCP roots 是范围提示，不是强安全边界。
真正的文件隔离必须由控制层、sandbox 或 worktree 执行。
```

#### OpenTelemetry

来源：

- https://opentelemetry.io/docs/concepts/signals/

值得吸收统一术语：

```text
trace   = 一次完整任务执行
span    = 一次 agent run / tool call / gate check
log     = 结构化事件
metric  = drift、耗时、失败率、重试次数
baggage = task_id / run_id / agent_id / approval_id
```

第一阶段继续使用 JSONL，只预留 correlation id。等 dashboard 或分布式 adapter 出现后，再考虑接 OpenTelemetry exporter。

### 第三优先级：安全增强

#### GitHub Environments / Vault Dynamic Secrets

来源：

- https://docs.github.com/actions/deployment/targeting-different-environments
- https://developer.hashicorp.com/hcp/docs/vault-secrets/dynamic-secrets

值得吸收：

- 高风险能力在审批前不可见、不可用。
- 权限按任务临时授予，并带 TTL。
- 执行结束或 session 终止后自动撤销。
- agent 不应默认持有长期凭据。

落地形式：

```text
capability lease:
  subject
  permissions
  resources
  task_id
  issued_at
  expires_at
  approved_by
  revoked_at
```

### Schema 工具选择

第一阶段直接使用 JSON Schema 校验：

```text
task.json
events.jsonl
control.json
assignment.json
agent-output.json
```

来源：

- https://json-schema.org/understanding-json-schema/reference/object
- https://cuelang.org/docs/

边界严格的对象默认关闭未知字段：

```json
{"unevaluatedProperties": false}
```

CUE 适合后续做配置组合、约束推导和 policy 校验，但不是 MVP 必需依赖。

### 暂时不要引入

第一阶段不要部署或强绑定：

```text
Temporal server
OPA / Cedar runtime
OpenTelemetry Collector
Vault
完整 in-toto 签名链
```

先把字段、事件和 trait 边界留出来。只有实际需求出现后，再把对应能力接入。

---

## 许可证提醒

- Trellis：AGPL-3.0。
- Spec-Driven Develop：MIT。
- GitHub Spec Kit：MIT。

如果复制 Trellis 代码、模板、脚本，新仓库大概率应按 AGPL-3.0 处理。

如果只吸收思想并用 Rust 重新实现状态机与模板，许可证空间更大。

---

## 最终判断

值得做 Rust 版，但方向应是：

```text
不是 Trellis rewrite
不是 Spec-Driven fork
而是 AI development control system
```

吸收 Trellis 的：

```text
task / archive / spec / journal / context manifest
```

吸收 Spec-Driven 的：

```text
闭环控制论 / drift / S.U.P.E.R / replan
```

再用 OMP 做执行器。这样比简单合并两个项目更干净、更强。

---

## 子智能体设计

子智能体必须是一等公民，不是简单“并发提示词”。它们应被控制系统调度、观测、记录，并纳入 drift 计算。

### 基本原则

```text
主控制器负责状态机、任务边界、调度与验收。
子智能体负责单一职责工作包。
所有子智能体输出必须结构化落盘。
所有子智能体行为都必须由控制层验证，并记录到 events.jsonl / telemetry.jsonl。
```

### 推荐子智能体角色

| 子智能体 | 职责 | 写权限 | 推荐模型 |
|---|---|---|---|
| `research-agent` | 代码库调查、外部资料、方案对比 | 只写 research/ | DeepSeek / GLM |
| `architecture-agent` | 架构分析、边界设计、S.U.P.E.R 评估 | 只写 research/ 与 design draft | GPT |
| `task-planner-agent` | 任务拆分、依赖图、执行顺序 | 只写 implement.md draft | GPT / GLM |
| `implement-agent` | 按单个任务包实现代码 | 可写代码 | GLM / DeepSeek |
| `test-agent` | 补测试、设计边界用例 | 可写测试 | DeepSeek / GLM |
| `review-agent` | diff review、风险识别、规范检查 | 只写 review report | GPT |
| `control-agent` | 汇总 telemetry、计算 drift、建议控制动作 | 只写 control.json | GPT / GLM |
| `archive-agent` | 归档、journal、摘要、commit notes | 只写归档与 workspace | MiMo / GLM |

### 子智能体目录

每个任务下保留子智能体输出：

```text
.trellis/tasks/<task>/
  agents/
    research-agent/
      output.md
      telemetry.json
    architecture-agent/
      output.md
      telemetry.json
    implement-agent-001/
      output.md
      touched-files.json
      telemetry.json
    review-agent/
      findings.json
      output.md
```

如果输出需要被后续执行读取，应再显式加入 `implement.jsonl` 或 `check.jsonl`：

```jsonl
{"file": ".trellis/tasks/<task>/agents/research-agent/output.md", "reason": "research findings for implementation"}
{"file": ".trellis/tasks/<task>/agents/review-agent/findings.json", "reason": "review findings for verification"}
```

### 子智能体调度协议

调度器输入：

```json
{
  "task_id": "05-28-auth-refactor",
  "agent": "implement-agent",
  "scope": ["src/auth/session.ts", "src/auth/token.ts"],
  "contract": "Implement token refresh without changing public API.",
  "context": [
    "prd.md",
    "design.md",
    ".trellis/spec/architecture/super.md"
  ],
  "write_policy": "code_and_tests",
  "acceptance": [
    "existing auth tests pass",
    "new refresh expiry tests pass",
    "no new circular dependencies"
  ]
}
```

子智能体输出必须包含：

```json
{
  "status": "completed | blocked | failed",
  "summary": "short factual summary",
  "files_touched": [],
  "tests_run": [],
  "risks": [],
  "open_questions": [],
  "telemetry": {
    "estimated_effort": 1,
    "actual_effort": 2,
    "scope_delta": 0.1,
    "unplanned_dependencies": [],
    "super_violations": 0
  }
}
```

### 并发策略

可以并发：

- 多个只读 research-agent。
- 互不重叠文件的 implement-agent。
- test-agent 与 review-agent 在实现完成后并行。

不应并发：

- 两个子智能体写同一文件。
- 架构边界未定时启动实现。
- control-agent 与正在写代码的 implement-agent 同时更新 `control.json`。

### 与控制闭环的关系

子智能体是控制系统里的 actuator / sensor hybrid：

```text
Actuator：执行具体任务包。
Sensor：报告实际变更、测试结果、偏差、风险。
Observer：由主控制器汇总所有子智能体 telemetry。
Controller：根据汇总 drift 决定 continue / annotate / replan / stop。
```

每个子智能体完成后追加事件：

```jsonl
{"type":"agent_started","agent":"implement-agent","task":"05-28-auth-refactor","time":"..."}
{"type":"agent_completed","agent":"implement-agent","status":"completed","telemetry":{"scope_delta":0.1}}
{"type":"drift_updated","score":0.27,"action":"annotate_next_task"}
```

### Rust MVP 增补命令

```text
control agent list
control agent run <agent> --task <task>
control agent report <task>
control agent telemetry <task>
control schedule plan <task>
control schedule run <task>
```

### OMP 集成方式

Rust 控制层不直接实现模型调用。它生成结构化 assignment，然后交给 OMP 的 subagent 能力执行。

```text
control schedule plan <task>  -> 生成 agent assignments
OMP task tool                 -> 并发执行
control agent ingest          -> 读取不可信输出，验证后写入 evidence 与 canonical events
control drift compute         -> 计算下一步控制动作
```

第一个正式 adapter 是 `manual`，先验证 assignment 协议。自动执行器第一版只需要支持 OMP。Codex / Claude / OpenCode 后续再做 adapter。

---

## 还需要补充的关键设计点

### 1. 事件溯源优先

不要只保存当前状态。所有状态变化都应先写事件，再由事件折叠出当前 `task.json` / `control.json`。

```text
events.jsonl = append-only truth
task.json    = materialized view
control.json = materialized control state
telemetry    = 带来源标记的不可信 evidence
```

外部不能直接写 canonical event。日常命令必须通过领域操作追加事件，不能把任意 JSON append 作为普通入口。

示例事件：

```jsonl
{"type":"task_created","task":"05-28-auth-refactor","by":"shaob","time":"..."}
{"type":"agent_started","agent":"research-agent","task":"05-28-auth-refactor","time":"..."}
{"type":"telemetry_recorded","source":"test-agent","tests_failed":2,"time":"..."}
{"type":"control_action_selected","action":"replan_remaining_tasks","drift_score":0.43,"time":"..."}
{"type":"task_archived","task":"05-28-auth-refactor","archive_path":"archive/2026-05/05-28-auth-refactor","time":"..."}
```

这样以后可以做：

```text
control replay <task>
control audit <task>
control explain-drift <task>
```

### 2. Schema version 与迁移

所有机器可读文件都必须带 schema version：

```json
{
  "schema": "control.task.v1",
  "id": "05-28-auth-refactor",
  "status": "planning"
}
```

Rust CLI 提供：

```text
control doctor
control migrate
control validate
```

否则一旦任务文件格式演进，旧任务会不可读。

### 3. Git / worktree 隔离

大型并发任务必须支持 worktree，避免子智能体互相踩文件。

worktree 只负责隔离变更和降低冲突，不是安全边界。它仍然共享 Git 元数据，也不能限制仓库外访问。真正的强制边界必须由控制层 policy 与 sandbox 执行。

推荐策略：

```text
单任务小改动：当前工作树
多 agent 并发实现：每个 implement-agent 一个 worktree
review/test：基于合并候选分支运行
```

任务字段预留：

```json
{
  "branch": "task/05-28-auth-refactor",
  "base_branch": "main",
  "worktrees": {
    "implement-agent-001": "../.worktrees/05-28-auth-refactor-impl-001"
  }
}
```

### 4. 验收门禁必须机器可执行

`prd.md` 可以写人类语言，但验收不能只靠文字。每个任务应有可执行 gate：

```json
{
  "gates": [
    {"type": "command", "cmd": "cargo test -p control-core"},
    {"type": "command", "cmd": "cargo clippy -p control-core -- -D warnings"},
    {"type": "lsp", "target": "changed_files"},
    {"type": "review", "agent": "review-agent", "severity_block": ["P0", "P1"]}
  ]
}
```

控制系统只在 gates 通过后允许：

```text
in_progress -> review -> completed
```

### 5. 安全与权限模型

子智能体必须有明确权限边界：

```text
read_only
write_research
write_tests
write_code_scoped
write_code_any
git_commit
network
```

默认策略：

- research-agent：`read_only + write_research`
- implement-agent：`write_code_scoped`
- test-agent：`write_tests`
- review-agent：`read_only`
- archive-agent：`write_workspace`
- 没有任何子智能体默认拥有 `git_commit`

### 6. Adapter 边界

Rust 控制层不要绑定某个 agent CLI 的内部实现。定义统一 adapter trait：

```rust
trait AgentBackend {
    fn capabilities(&self) -> Capabilities;
    fn run_assignment(&self, assignment: Assignment) -> AgentRun;
    fn collect_output(&self, run: AgentRun) -> AgentOutput;
}
```

第一批 backend：

```text
manual
omp
codex
claude
opencode
```

`manual` 是第一个正式 adapter，不是临时兜底：CLI 先生成结构化任务包，让人或任意 AI 工具执行并回填结果。这样可以在接入自动执行器前验证协议是否完整。

### 7. 人机协作断点

控制系统必须知道哪些动作需要人确认：

```text
删除文件
数据库迁移
公共 API 变更
依赖升级
安全策略变更
超过 drift 阈值的重规划
git commit / push
```

对应事件：

```jsonl
{"type":"human_approval_requested","reason":"public_api_change","time":"..."}
{"type":"human_approval_granted","by":"shaob","time":"..."}
```

### 8. 文档不要过载

吸收 SDD 时必须防止 Markdown 膨胀。建议三档任务模式：

| 模式 | 文件 |
|---|---|
| small | `task.json` + 简短 `notes.md` |
| medium | `prd.md` + `implement.md` |
| large | `prd.md` + `design.md` + `implement.md` + `research/` + control telemetry |

不要让小 bug 生成完整 PRD/design/research。

### 9. 可观测性

后续可以加一个本地 dashboard，但第一版先输出结构化报告：

```text
control status --json
control report <task>
control drift explain <task>
control agents report <task>
```

报告应回答：

```text
现在目标是什么？
已经做了什么？
偏差在哪里？
谁做出的判断？
哪些证据支持这个判断？
下一步为什么是这个动作？
```

### 10. 审计暂停与完成闸门

审计不是实现过程中的提示消息，而是权限状态变化：

```text
implement -> audit_hold -> deterministic audit
```

在 `audit_hold` 中，只允许读取文件、解释规则和运行 allowlist 内的离线 gate。实现者、reviewer 和 telemetry 的 PASS 只是 evidence；控制层必须独立检查固定审计矩阵、baseline manifest 和未决审批，再决定恢复、完成或停止。

固定审计矩阵至少覆盖：

```text
Schema 正例与反例
reducer 合法与非法转换
replay 一致性
Windows UNC / junction / symlink / root escape
受保护文件变更
依赖变化
required gate 变化
baseline 回退
```

### 11. 第一阶段真正的切入点

最小可落地不是“写完整平台”，而是先完成 `M0-M3`，替换 Trellis 的基础 task 能力并形成 manual 闭环。详细退出条件见：[ROADMAP.md](./ROADMAP.md)。

```text
control init
control task create/ready/start/submit/finish/archive
control context build
control status
control assignment create/export
control run ingest --adapter manual
control replay/reconcile/validate
```

低级事件入口不能作为 agent 或普通用户路径。外部只能提交 evidence，由控制层验证后生成 canonical event。

然后按真实 dogfood 结果再加：

```text
M4: OMP 单执行器隔离运行
M5: telemetry / drift / next-action
M6: 受限多智能体与 schedule
```
