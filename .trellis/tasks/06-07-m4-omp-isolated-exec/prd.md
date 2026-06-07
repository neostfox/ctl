# M4: OMP Single-Executor Isolated Execution

## Goal

OMP 可以执行单个 assignment，并且只能在 disposable worktree 中写入。主工作区只接受经过 apply gate 的 diff。

## Parent Task

M3 Manual 闭环 MVP（已完成）。M4 在 M3 的 assignment/evidence/audit 基础上增加：
- OMP adapter 直接消费 assignment 并在隔离 worktree 中执行
- worktree diff 必须经过 apply gate 才能合并到主工作区
- 高风险变更需要 step-up approval
- Capability lease 绑定 task/run/资源/动作/TTL

## Confirmed Facts

### M3 已有基础设施
- `assignment.json` 包含 objective、read_scope、write_allow、write_deny、risk_triggers、gates、contract、context_hashes、required_capabilities、acceptance
- `agent-output.json` 是手动 adapter 的 evidence 格式
- `events.jsonl` 是唯一真相源
- `reconcile` 可以从 events 重建所有投影
- Gate runner 支持 cargo_fmt_check、cargo_check、cargo_test、cargo_clippy
- PathNormalizer 处理 Windows 路径规范化
- Completion interlock 检查 gate 通过、无未解决 rejected evidence
- OMP skill `.omp/skills/control-guard/SKILL.md` 已定义 M3 agent 循环

### OMP 执行环境
- OMP 是主执行器（read/search/find/edit/lsp/debug/browser/task 统一接口）
- OMP agent 通过 skill 文件指导自动执行 control 命令
- 当前 OMP 直接写主工作区，M4 必须改为 worktree 隔离

### 架构约束（from ARCHITECTURE_GUARDRAILS.md）
- `MODULE-004`: OMP 专有逻辑不能进入 `domain/` 或 `application/`
- `ADAPTER-004`: M4 才允许 OMP 单执行器，只能写 disposable worktree
- `ADAPTER-006`: 新 adapter 必须通过与 manual 相同的 contract tests
- `ADAPTER-007`: OMP RPC/ACP host execution 在控制层提供 wrapper 前只能用于只读检查

## Requirements

### R1: Adapter Trait 抽象
- 定义 `ExecutorAdapter` trait：`propose() → lease() → implement() → audit_hold()`
- `manual` adapter 和 `omp` adapter 都实现此 trait
- Trait 定义在 `src/adapters/` 层，不在 `domain/` 或 `application/`

### R2: Git Worktree 隔离
- `control workspace create --id <task_id>` 创建 git worktree
- Worktree 路径：`.trellis/tasks/<id>/worktree/`
- Worktree 基于当前 HEAD 创建干净分支
- OMP 在 worktree 中执行，主工作区不受影响

### R3: Workspace Diff 分析
- `control workspace diff --id <task_id>` 比较 worktree 与主分支
- Diff 分析必须识别：
  - 新增文件
  - 修改文件（在 write_allow 内 vs 越界）
  - 删除文件（高风险，需要 approval）
  - 依赖变化（Cargo.toml/Cargo.lock，高风险）
  - 公共 API / 安全策略变化（高风险）

### R4: Apply Gate
- `control workspace apply --id <task_id>` 将经过检查的 diff 合并到主工作区
- 拒绝条件：
  - 越界 diff（不在 write_allow 内）
  - 租约过期
  - 高风险变更未获 approval
  - 跨 task 租约
- 合并方式：`git checkout` 从 worktree 复制文件到主工作区

### R5: OMP Adapter
- `control run start --adapter omp --id <task_id>` 启动 OMP 执行
- 读取 `assignment.json`，在 worktree 中执行
- 执行完成自动生成 `agent-output.json`
- `control adapter capabilities omp` 报告 adapter 能力

### R6: Step-Up Approval
- `control approval request --id <task_id> --reason <reason> --scope <json>` 创建结构化 approval request
- `control approval grant --id <task_id> --request <request_id>` 批准
- `control approval deny --id <task_id> --request <request_id>` 拒绝
- 高风险变更类型：
  - 文件删除
  - Cargo.toml / Cargo.lock 变化
  - 公共 API 变化（pub fn/struct/enum 签名变化）
  - 安全相关文件变化（.omp/settings.json 等）
- Approval request 是结构化 JSON（变更类型、影响文件、具体 diff），不是自然语言

### R7: Capability Lease
- Lease 绑定：task_id、run_id、资源路径、动作类型、TTL、max_uses
- Lease 创建时检查：
  - 无跨 task 重叠写入
  - write_allow 内
  - TTL 有效
- Lease 使用时检查：
  - 未过期
  - 剩余 uses > 0
  - 资源在 write_allow 内
- Lease 失效条件：
  - TTL 过期
  - max_uses 耗尽
  - task 进入 terminal state（completed/cancelled/archived）

### R8: 恢复与失败处理
- OMP 中断后 `control status` 可以恢复上下文
- 中断的 worktree 可以继续或清理
- 清理 = 删除 worktree + 记录 `run_failed` 事件

## Acceptance Criteria

1. OMP 中断后可以恢复或明确失败
2. 越界 diff 被拒绝（有错误消息、命中的规则、解除方式）
3. 过期 lease 被拒绝
4. 跨 task lease 被拒绝
5. 重复 lease 被拒绝
6. 高风险变更未经 approval 不能 apply
7. Approval 只能批准结构化 request
8. cargo fmt --check
9. cargo check --locked --offline
10. cargo test --locked --offline
11. cargo clippy --locked --offline -- -D warnings
12. cargo run --locked --offline -- architecture check
13. OMP adapter 通过与 manual adapter 相同的 contract tests
14. 至少用 M3/M4 完成 20 个真实任务（M3 已有 10 个，M4 再做 10 个）

## Out of Scope

- 并发写入（M6）
- 自动合并
- 其他执行器 adapter（Codex/Claude/OpenCode）
- 长期密钥注入
- AgentRun 独立 aggregate（M6 前不拆）
- Daemon / database / network service
- drift / telemetry / next-action

## Open Questions

1. **Worktree 生命周期**：worktree 在 task archive 时自动清理，还是保留？
   - 推荐：archive 时自动清理，除非 `--keep-worktree`
2. **OMP 启动方式**：`control run start --adapter omp` 是 fork OMP 进程，还是生成指令让 OMP skill 执行？
   - 推荐：生成指令（写入 run manifest），OMP skill 读取并执行。不 fork 进程。
3. **Lease 持久化**：lease 状态存在 events.jsonl 还是单独文件？
   - 推荐：lease 作为事件存入 events.jsonl（lease_created、lease_used、lease_expired），保持单一真相源。
4. **Approval 超时**：approval request 是否需要超时自动过期？
   - 推荐：是。默认 24 小时，可配置。
