# M3 Dogfood: 10 real small tasks

## Goal

Dogfood验证M3手动闭环：用 `control` CLI 完成10个真实 small 任务，验证端到端生命周期、中断恢复、审计一致性和越界拦截。通过后决定是否进入 M4。

## Background

M3 代码实现已完成并提交（`ec83962`），但 ROADMAP 明确要求：

> 使用 M3 完成至少 10 个真实 small 任务。

> M4 以后必须根据 dogfood 结果决定是否继续。

## Dogfood 任务清单

每个任务必须是真实的代码变更（非空操作），覆盖以下场景：

| # | 任务类型 | 验证重点 |
|---|---------|---------|
| 1 | 添加一个新的 gate 模板 | 基本生命周期：create → ready → start → context → implement → gate → submit → audit → finish → archive |
| 2 | 修改 reducer 添加新字段 | assignment export + run ingest 端到端 |
| 3 | 添加一个新的 CLI 子命令 | 中断恢复：start 后中断，用 status 恢复 |
| 4 | 修改 schema 添加新约束 | 越界拦截：故意写 write_deny 中的文件 |
| 5 | 添加测试用例 | evidence rejected 场景：提交包含越界 touched_files 的 result |
| 6 | 修改帮助文本/文档 | 重放后 audit 一致性验证 |
| 7 | 重构内部函数 | dry-run：所有写操作支持预检 |
| 8 | 修改边界检查逻辑 | 多文件 scope：read_scope 和 write_allow 包含多个目录 |
| 9 | 添加 infrastructure 工具函数 | cancel 中途取消任务生命周期 |
| 10 | 修改 OMP skill 文本 | reopen 场景：submit → reopen → 重新实现 → submit → finish |

## Acceptance Criteria

- [ ] 10 个真实 small 任务全部从 create 到 archive 端到端通过
- [ ] 每个任务的 `control audit` 报告 completion_interlock.verdict = "completed"
- [ ] 至少 1 个任务验证中断恢复（start 后停顿，用 status 恢复上下文）
- [ ] 至少 1 个任务验证越界文件被拒绝（boundary_violation_recorded 或 ingest 拒绝）
- [ ] 至少 1 个任务验证 evidence rejected 场景
- [ ] 至少 1 个任务验证 cancel 生命周期
- [ ] 至少 1 个任务验证 reopen 场景
- [ ] 所有 10 个任务完成后，`control report` 列出正确状态
- [ ] 全部完成后运行 `cargo test` + `cargo clippy` + `architecture check` 仍然通过
- [ ] 记录 dogfood 过程中发现的 UX 问题或 bug，作为 M4 输入

## Notes

- 每个 small 任务应能在 30 秒内创建并启动（体验标准）
- 记录每个任务实际耗时和遇到的问题
- Dogfood 期间发现的 bug 直接修复，不需要单独建任务
- 最终产出一份 dogfood 报告，记录：完成的任务列表、每步耗时、发现的 UX 问题、建议改进
