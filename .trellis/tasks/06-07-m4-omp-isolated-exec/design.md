# M4 Design: OMP Single-Executor Isolated Execution

## Architecture

### New Modules

```
src/
  adapters/
    mod.rs              — ExecutorAdapter trait 定义
    manual/mod.rs       — manual adapter（已存在）
    omp/mod.rs          — OMP adapter 实现
  infrastructure/
    workspace/
      mod.rs            — git worktree 创建/删除/diff/apply
    lease/
      mod.rs            — capability lease 管理（事件溯源）
  domain/
    lease.rs            — Lease 值对象 + lease 事件类型
    approval.rs         — Approval 值对象 + approval 事件类型
```

### Event Schema Additions

新增事件类型（需加入 schema enum）：

| 事件类型 | 用途 |
|---|---|
| `workspace_created` | worktree 创建 |
| `workspace_cleaned` | worktree 清理 |
| `workspace_diff_computed` | diff 分析结果 |
| `workspace_applied` | diff 合并到主工作区 |
| `run_started` | OMP adapter 执行启动 |
| `run_completed` | 执行完成 |
| `run_failed` | 执行失败 |
| `lease_created` | 租约创建 |
| `lease_used` | 租约使用（递减 max_uses） |
| `lease_expired` | 租约过期（TTL 或 max_uses 耗尽） |
| `lease_revoked` | 租约撤销 |
| `approval_requested` | 审批请求 |
| `approval_granted` | 审批批准 |
| `approval_denied` | 审批拒绝 |
| `approval_expired` | 审批超时过期 |

### Data Flow

```
control run start --adapter omp --id <task>
  → 检查 task 状态 = in_progress
  → 检查无其他 active run
  → workspace create（git worktree）
  → lease create（绑定 task、run、资源、动作、TTL）
  → 生成 run manifest → .trellis/tasks/<id>/run-manifest.json
  → 记录 run_started 事件

OMP skill 读取 run-manifest.json:
  → 在 worktree 中执行 assignment
  → 生成 agent-output.json

control workspace diff --id <task>
  → 比较 worktree 与 HEAD
  → 分析越界、高风险变更
  → 对高风险变更自动创建 approval request
  → 记录 workspace_diff_computed 事件

control approval grant --id <task> --request <req_id>
  → 记录 approval_granted 事件

control workspace apply --id <task>
  → 检查：所有 diff 在 write_allow 内
  → 检查：高风险变更已获 approval
  → 检查：lease 有效
  → 从 worktree 复制文件到主工作区
  → 记录 workspace_applied 事件

control run ingest --adapter omp --id <task> --result agent-output.json
  → 与 manual adapter 相同的 evidence 验证流程
```

### Run Manifest Schema

```json
{
  "schema": "control.run-manifest.v1",
  "run_id": "uuid",
  "task_id": "string",
  "adapter": "omp",
  "assignment_path": ".trellis/tasks/<id>/assignment.json",
  "worktree_path": ".trellis/tasks/<id>/worktree",
  "lease_id": "uuid",
  "write_allow": ["src/..."],
  "write_deny": [],
  "gates": ["cargo_check", "cargo_test"],
  "created_at": "ISO8601"
}
```

### Lease Lifecycle

```
lease_created (max_uses=N, ttl=T)
  → lease_used (max_uses--) × N
  → lease_expired (max_uses=0 或 now > created_at + ttl)
  或
  → lease_revoked (task completed/cancelled)
```

### Approval Lifecycle

```
approval_requested (reason, scope, ttl)
  → approval_granted (human approves)
  或
  → approval_denied (human rejects)
  或
  → approval_expired (now > requested_at + ttl)
```

### ExecutorAdapter Trait

```rust
trait ExecutorAdapter {
    fn adapter_name(&self) -> &str;
    fn capabilities(&self) -> serde_json::Value;
    fn prepare_run(&self, task_id: &str, worktree: &Path) -> Result<RunManifest>;
    fn validate_output(&self, output: &serde_json::Value) -> Result<()>;
}
```

定义在 `src/adapters/mod.rs`，不在 `domain/`。

### High-Risk Change Detection

文件 diff 分类为高风险的条件：
1. 文件被删除（worktree 中存在，主分支中不存在）
2. `Cargo.toml` 或 `Cargo.lock` 变化
3. `pub fn`/`pub struct`/`pub enum` 签名变化（需要简单的 AST diff 或正则检测）
4. 安全相关文件：`.omp/`、`.trellis/spec/`、`schemas/`
5. `README.md` 变化（审核级别较低，可配置）

M4 MVP 实现方案：只检测 1-4，不做 AST diff。使用简单规则：
- 删除 = worktree 中没有但 HEAD 中有
- 依赖变化 = Cargo.toml 或 Cargo.lock 在 diff 中
- 安全文件 = 路径匹配 protected_paths 列表

## Compatibility / Migration

- 所有新事件类型需要加入 `control.event-envelope.v1.schema.json`
- `TaskState` 需要扩展：`active_run`、`leases`、`pending_approvals` 字段
- 现有 `manual` adapter 不受影响（共享 ingest 路径）
- `control run start` 是新命令，不影响现有 `control run ingest`

## Key Tradeoffs

1. **Run manifest vs fork 进程**：manifest 模式解耦控制层和执行器，OMP 可以按自己的节奏执行。代价是需要 OMP skill 主动读取 manifest。
2. **Lease 事件溯源 vs 独立文件**：事件溯源保持单一真相源但 lease 查询需要重放。M4 任务数量小，重放开销可忽略。
3. **Worktree 在 .trellis/tasks/ 下 vs 独立位置**：放在 task 目录下方便管理，但增加目录大小。Archive 时自动清理。
