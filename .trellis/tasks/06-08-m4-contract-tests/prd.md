# M4 Verification: Contract Tests + Acceptance Criteria

## Goal

验证 M4 实现满足全部 14 条验收标准，补齐缺失的 AC4（跨 task lease 拒绝），完成 AC14（dogfood 10 个真实任务）。

## Parent Task

M4: OMP Single-Executor Isolated Execution（骨架已实现，5 个机械门槛通过）。

## Confirmed Facts

### 已完成
- Schema: 15 个新事件类型 + payload 定义
- Domain: LeaseState, ApprovalState, RunInfo, 扩展的 TaskState + reducer
- Infrastructure: workspace 模块（git worktree 管理）
- Adapters: ExecutorAdapter trait + OmpAdapter 实现
- Application: 12 个 M4 命令方法
- CLI: workspace/approval/adapter/run start 命令
- Fixtures: reducer_m4_lifecycle.jsonl（18 事件完整生命周期）
- 104 tests pass, clippy clean, architecture check clean

### 未完成
- AC1-6: 有实现但无可执行测试断言
- AC4: 跨 task lease 拒绝未实现
- AC13: OMP adapter contract test 未写
- AC14: 10 个真实 M4 dogfood 任务

## Requirements

### R1: Reducer 级别验收测试
为 AC1-6 在 `src/domain/audit_matrix.rs` 中写测试：
- AC1: run_started → run_failed 清除 active_run
- AC2: out-of-scope workspace_apply 在 application 层被拒绝
- AC3: lease_used 在 remaining_uses=0 时拒绝
- AC5: 重复 lease_id 被拒绝
- AC6: workspace_apply 中高风险未 approval 被拒绝（application 层）

### R2: 跨 Task Lease 拒绝 (AC4)
- `run_start` 中检查所有 task 的 active lease，拒绝 write scope 重叠
- 需要遍历 `.trellis/tasks/*/events.jsonl` 或使用 reconcile 获取全局状态

### R3: OMP Adapter Contract Tests (AC13)
- `validate_output` 合法 agent-output.json 通过
- `validate_output` 非法 source 被拒绝
- `validate_output` 缺少 touched_files 被拒绝
- `prepare_run` 生成正确的 RunManifest

### R4: Dogfood 10 个真实任务 (AC14)
- 通过 `control run start --adapter omp` 执行 10 个 small 任务
- 记录每个任务的 task_id、执行结果、发现的问题
- 验证完整流程：create → ready → start → workspace → run → diff → apply → gates → submit → finish → archive

## Acceptance Criteria

1. AC1-AC7 全部有可执行测试通过
2. AC4 跨 task lease 重叠被拒绝
3. AC13 OMP adapter contract tests 通过
4. AC14 完成 10 个真实 dogfood 任务
5. cargo fmt/check/test/clippy/architecture check 全部通过
6. 测试数量不少于 110（当前 104）

## Out of Scope

- M5/M6 功能
- 并发执行
- 自动 reviewer
- drift/telemetry

## Open Questions

1. AC4 跨 task 检查是否需要全局状态？推荐：`run_start` 时 `reconcile` 所有 task 的 lease，检查 write scope 重叠。
2. Dogfood 任务来源：推荐用当前项目本身的改进任务（lint 警告修复、文档补充等）。
